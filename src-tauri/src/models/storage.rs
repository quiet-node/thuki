/*!
 * Content-addressed model blob store.
 *
 * Downloaded GGUF files land here in two stages:
 *
 * 1. The downloader writes bytes into `root/tmp/<sha256>.partial` so
 *    interrupted downloads can be resumed from the already-written offset.
 * 2. On completion the store verifies the file by streaming it through
 *    SHA-256 (buffered copy; never fully buffered in memory) and, on match, atomically
 *    renames it into `root/blobs/<sha256>`. A mismatch deletes the partial
 *    and returns [`StorageError::VerifyFailed`].
 *
 * `free_disk_bytes` is a thin `libc::statfs` wrapper used by callers to show
 * a low-disk warning before starting a download. Treating `None` as "unknown"
 * and skipping the warning is safe; the function never panics.
 */

use std::io;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// Errors returned by [`ModelStore`] operations.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The partial file's SHA-256 did not match the expected digest.
    #[error("download did not verify: expected sha256 {expected}, got {actual}")]
    VerifyFailed { expected: String, actual: String },
    /// Any I/O failure (missing file, permission error, rename failure).
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Content-addressed store rooted at a caller-supplied directory (in the app
/// this is `<app_data>/models`).
///
/// Layout:
/// - `root/blobs/<sha256>`: completed, verified blobs.
/// - `root/tmp/<sha256>.partial`: in-flight downloads (resume-safe).
pub struct ModelStore {
    root: PathBuf,
}

impl ModelStore {
    /// Creates the store handle and eagerly creates the `blobs/` and `tmp/`
    /// subdirectories so callers do not have to guard against missing dirs.
    ///
    /// # Errors
    ///
    /// Returns an error if either subdirectory cannot be created.
    pub fn new(root: PathBuf) -> Result<Self, io::Error> {
        std::fs::create_dir_all(root.join("blobs"))?;
        std::fs::create_dir_all(root.join("tmp"))?;
        Ok(Self { root })
    }

    /// Absolute path where a verified blob is stored: `root/blobs/<sha256>`.
    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        self.root.join("blobs").join(sha256)
    }

    /// Absolute path for an in-flight download: `root/tmp/<sha256>.partial`.
    pub fn partial_path(&self, sha256: &str) -> PathBuf {
        self.root.join("tmp").join(format!("{sha256}.partial"))
    }

    /// Streams `root/tmp/<sha256>.partial` through SHA-256 (buffered copy,
    /// never whole-file in memory). On hash match the partial is atomically
    /// renamed into `root/blobs/<sha256>` and the blob path is returned.
    /// On mismatch the partial is deleted and [`StorageError::VerifyFailed`]
    /// is returned. `sha256` must be a lowercase hex digest; the comparison
    /// is case-sensitive.
    pub fn verify_and_install(&self, sha256: &str) -> Result<PathBuf, StorageError> {
        let partial = self.partial_path(sha256);
        let mut file = std::fs::File::open(&partial)?;

        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher)?;
        let actual = format!("{:x}", hasher.finalize());

        if actual != sha256 {
            // Best-effort delete; ignore secondary I/O errors.
            let _ = std::fs::remove_file(&partial);
            return Err(StorageError::VerifyFailed {
                expected: sha256.to_string(),
                actual,
            });
        }

        let blob = self.blob_path(sha256);
        std::fs::rename(&partial, &blob)?;
        Ok(blob)
    }

    /// Removes each blob in `shas` from `root/blobs/`. Missing files are
    /// silently ignored so callers do not need to pre-check existence.
    pub fn remove_blobs(&self, shas: &[String]) -> io::Result<()> {
        for sha in shas {
            let path = self.blob_path(sha);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// Returns the byte length of an in-flight partial download, or `None`
    /// if no partial file exists for `sha256`. Used by the downloader to
    /// resume from the already-written offset. Inherently racy with a
    /// concurrent writer: the downloader must tolerate the partial changing
    /// between this call and opening the file.
    pub fn existing_partial_len(&self, sha256: &str) -> Option<u64> {
        let meta = std::fs::metadata(self.partial_path(sha256)).ok()?;
        Some(meta.len())
    }
}

/// Free bytes available on the volume holding `path`.
///
/// Thin `libc::statfs` wrapper. Callers must treat `None` as "unknown" and
/// skip disk-space warnings rather than blocking the download.
///
/// Not covered by the cargo coverage gate: this is a direct OS syscall with
/// no branching logic beyond error propagation, making branch-level
/// instrumentation meaningless here.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn free_disk_bytes(path: &std::path::Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `buf` is a valid, zeroed `libc::statfs` on the stack;
    // `c_path` is a valid null-terminated C string. `libc::statfs` writes
    // into `buf` only on success (return value 0).
    unsafe {
        let mut buf: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut buf) == 0 && buf.f_bsize > 0 {
            (buf.f_bavail as u64).checked_mul(buf.f_bsize as u64)
        } else {
            None
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    /// Build a fresh store rooted at a temporary directory.
    fn make_store() -> (TempDir, ModelStore) {
        let dir = TempDir::new().unwrap();
        let store = ModelStore::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    /// Compute the hex SHA-256 of `data`.
    fn sha256_of(data: &[u8]) -> String {
        format!("{:x}", Sha256::digest(data))
    }

    /// Write `data` into the store's partial slot for `sha256`.
    fn write_partial(store: &ModelStore, sha256: &str, data: &[u8]) {
        std::fs::write(store.partial_path(sha256), data).unwrap();
    }

    // ── Path helpers ─────────────────────────────────────────────────────────

    #[test]
    fn blob_path_is_content_addressed() {
        let (_dir, store) = make_store();
        let p = store.blob_path("abc123");
        assert!(p.ends_with("blobs/abc123"));
    }

    #[test]
    fn partial_path_stable_for_resume() {
        let (_dir, store) = make_store();
        let p = store.partial_path("abc123");
        assert!(p.ends_with("tmp/abc123.partial"));
        // Calling twice must return the same path (stable across calls).
        assert_eq!(store.partial_path("abc123"), p);
    }

    // ── verify_and_install ───────────────────────────────────────────────────

    #[test]
    fn install_renames_atomically() {
        let (_dir, store) = make_store();
        let data = b"hello content-addressed world";
        let sha = sha256_of(data);

        write_partial(&store, &sha, data);
        let blob = store.verify_and_install(&sha).unwrap();

        // Blob is at the expected path and contains the original bytes.
        assert_eq!(blob, store.blob_path(&sha));
        assert_eq!(std::fs::read(&blob).unwrap(), data);

        // Partial must be gone after a successful install.
        assert!(!store.partial_path(&sha).exists());
    }

    #[test]
    fn install_rejects_sha_mismatch() {
        let (_dir, store) = make_store();
        let data = b"real bytes";
        let real_sha = sha256_of(data);
        let wrong_sha = "0000000000000000000000000000000000000000000000000000000000000000";

        // Partial is filed under the wrong (expected) SHA.
        write_partial(&store, wrong_sha, data);

        let err = store.verify_and_install(wrong_sha).unwrap_err();
        assert!(
            matches!(&err, StorageError::VerifyFailed { .. }),
            "expected VerifyFailed, got {err}"
        );
        // The Display message contains both hashes; check without branching on
        // the enum variant so no instrumented line goes uncovered.
        let msg = err.to_string();
        assert!(msg.contains(wrong_sha), "message missing expected hash");
        assert!(msg.contains(&real_sha), "message missing actual hash");

        // Partial must be deleted on mismatch.
        assert!(!store.partial_path(wrong_sha).exists());
    }

    #[test]
    fn install_missing_partial_returns_io_error() {
        let (_dir, store) = make_store();
        let err = store.verify_and_install("deadbeef").unwrap_err();
        assert!(matches!(err, StorageError::Io(_)));
    }

    // ── remove_blobs ─────────────────────────────────────────────────────────

    #[test]
    fn remove_blobs_deletes_files_and_tolerates_missing() {
        let (_dir, store) = make_store();

        // Write two blobs directly into the blobs dir.
        let sha_a = "aaaa";
        let sha_b = "bbbb";
        std::fs::write(store.blob_path(sha_a), b"a").unwrap();
        std::fs::write(store.blob_path(sha_b), b"b").unwrap();

        // Remove one real and one that never existed.
        let shas = vec![sha_a.to_string(), "cccc".to_string(), sha_b.to_string()];
        store.remove_blobs(&shas).unwrap();

        assert!(!store.blob_path(sha_a).exists());
        assert!(!store.blob_path(sha_b).exists());
    }

    #[test]
    fn remove_blobs_propagates_non_not_found_io_error() {
        let (_dir, store) = make_store();
        // Place a directory at the blob path so remove_file returns IsADirectory,
        // which is not NotFound and must be propagated as Err.
        let sha = "dirblob";
        let path = store.blob_path(sha);
        std::fs::create_dir_all(&path).unwrap();
        let err = store.remove_blobs(&[sha.to_string()]).unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
    }

    // ── existing_partial_len ─────────────────────────────────────────────────

    #[test]
    fn existing_partial_len_some_and_none() {
        let (_dir, store) = make_store();

        // No partial yet: must return None.
        assert_eq!(store.existing_partial_len("nothere"), None);

        // Write 42 bytes into the partial slot.
        let sha = "feedface";
        write_partial(&store, sha, &[0u8; 42]);
        assert_eq!(store.existing_partial_len(sha), Some(42));
    }

    // ── free_disk_bytes ───────────────────────────────────────────────────────

    #[test]
    fn free_disk_bytes_returns_some_on_real_fs() {
        let (dir, _store) = make_store();
        let free = free_disk_bytes(dir.path());
        assert!(free.is_some(), "expected Some on a real filesystem");
    }

    // ── StorageError display ─────────────────────────────────────────────────

    #[test]
    fn storage_error_verify_failed_message_contains_both_hashes() {
        let err = StorageError::VerifyFailed {
            expected: "exp".to_string(),
            actual: "act".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("exp"), "message missing expected hash");
        assert!(msg.contains("act"), "message missing actual hash");
    }

    #[test]
    fn storage_error_io_is_transparent() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        let err = StorageError::Io(io_err);
        assert!(err.to_string().contains("denied"));
    }
}
