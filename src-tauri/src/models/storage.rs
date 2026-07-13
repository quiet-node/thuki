/*!
 * Content-addressed model blob store.
 *
 * Downloaded GGUF files land here in two stages:
 *
 * 1. The downloader writes bytes into `root/tmp/<sha256>.partial` so
 *    interrupted downloads can be resumed from the already-written offset.
 * 2. On completion the file's SHA-256 is checked against the expected digest.
 *    The downloader hashes bytes as they stream in; a full-length partial that
 *    was never streamed is read back through SHA-256 here. On match the partial
 *    is atomically renamed into `root/blobs/<sha256>`; a mismatch deletes the
 *    partial and returns [`StorageError::VerifyFailed`].
 *
 * `free_disk_bytes` is a thin `libc::statfs` wrapper used by callers to show
 * a low-disk warning before starting a download. Treating `None` as "unknown"
 * and skipping the warning is safe; the function never panics.
 */

use std::io;
use std::path::PathBuf;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::HfGgufPart;
use crate::config::defaults::BLOB_HASH_BUFFER_BYTES;

/// Renders a SHA-256 digest as a lowercase hex string.
pub(crate) fn hex_digest(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Adapts a [`Digest`] hasher to [`io::Write`] so it can be used as a
/// [`ModelStore::feed_partial`] sink. sha2 0.11 dropped hashers' own `Write`
/// impl; this forwards each write straight into [`Digest::update`].
pub(crate) struct HashWriter<'a, D: Digest>(pub &'a mut D);

impl<D: Digest> io::Write for HashWriter<'_, D> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

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

/// A paused download's running SHA-256, kept in memory so an in-session resume
/// can continue it instead of re-reading the whole on-disk prefix back through
/// SHA-256. `hasher` has consumed exactly `len` bytes of the partial `sha256`.
struct SuspendedHash {
    sha256: String,
    len: u64,
    hasher: Sha256,
}

/// Content-addressed store rooted at a caller-supplied directory (in the app
/// this is `<app_data>/models`).
///
/// Layout:
/// - `root/blobs/<sha256>`: completed, verified blobs.
/// - `root/tmp/<sha256>.partial`: in-flight downloads (resume-safe).
pub struct ModelStore {
    root: PathBuf,
    /// Running hash of the single in-flight download kept across a pause so an
    /// in-session resume continues it rather than re-hashing the prefix. Holds
    /// at most one entry (one download at a time); a later save overwrites it.
    suspended_hash: Mutex<Option<SuspendedHash>>,
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
        Ok(Self {
            root,
            suspended_hash: Mutex::new(None),
        })
    }

    /// Remembers a paused download's running `hasher` (which has consumed
    /// exactly `len` bytes of the partial for `sha256`) so an in-session resume
    /// can continue it. At most one is kept; a later save overwrites it.
    pub fn save_suspended_hash(&self, sha256: &str, len: u64, hasher: Sha256) {
        *self.suspended_hash.lock().unwrap() = Some(SuspendedHash {
            sha256: sha256.to_string(),
            len,
            hasher,
        });
    }

    /// Takes the kept running hash for `sha256` when it stands exactly at the
    /// resume offset `len`. Clears the slot either way, so a stale entry never
    /// lingers; returns the hasher to continue, or `None` to re-hash from disk.
    pub fn take_suspended_hash(&self, sha256: &str, len: u64) -> Option<Sha256> {
        match self.suspended_hash.lock().unwrap().take() {
            Some(s) if s.sha256 == sha256 && s.len == len => Some(s.hasher),
            _ => None,
        }
    }

    /// Absolute path where a verified blob is stored: `root/blobs/<sha256>`.
    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        self.root.join("blobs").join(sha256)
    }

    /// Absolute path for an in-flight download: `root/tmp/<sha256>.partial`.
    pub fn partial_path(&self, sha256: &str) -> PathBuf {
        self.root.join("tmp").join(format!("{sha256}.partial"))
    }

    /// Streams the existing partial for `sha256` into `sink` using a large read
    /// buffer (never whole-file in memory). Used to hash a full-length partial
    /// that was never streamed live, and to seed an incremental hasher with the
    /// bytes already on disk before a resumed download appends the rest.
    ///
    /// `cancelled` is polled once per read buffer (every
    /// [`BLOB_HASH_BUFFER_BYTES`]); when it returns true the read stops early,
    /// so a pause during a multi-GB resume re-hash lands promptly instead of
    /// after the whole prefix is read. A cancelled read leaves a partial sink;
    /// callers that cancel discard the sink (the running hash) entirely.
    pub fn feed_partial<W: io::Write>(
        &self,
        sha256: &str,
        sink: &mut W,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<()> {
        use io::Read;
        let mut file = std::fs::File::open(self.partial_path(sha256))?;
        let mut buf = vec![0u8; BLOB_HASH_BUFFER_BYTES];
        while !cancelled() {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            sink.write_all(&buf[..n])?;
        }
        Ok(())
    }

    /// Finalizes a downloaded partial whose SHA-256 `actual` is already known
    /// (hashed live during the download, or by [`Self::verify_and_install`]). On
    /// match the partial is atomically renamed into `root/blobs/<sha256>` and
    /// the blob path is returned; on mismatch the partial is deleted and
    /// [`StorageError::VerifyFailed`] is returned. `sha256` must be a lowercase
    /// hex digest; the comparison is case-sensitive.
    pub fn install_if_matches(&self, sha256: &str, actual: &str) -> Result<PathBuf, StorageError> {
        let partial = self.partial_path(sha256);
        if actual != sha256 {
            // Best-effort delete; ignore secondary I/O errors.
            let _ = std::fs::remove_file(&partial);
            return Err(StorageError::VerifyFailed {
                expected: sha256.to_string(),
                actual: actual.to_string(),
            });
        }
        let blob = self.blob_path(sha256);
        std::fs::rename(&partial, &blob)?;
        Ok(blob)
    }

    /// Reads `root/tmp/<sha256>.partial` back through SHA-256 and installs it.
    /// Used for a full-length partial whose hash was never computed during a
    /// live download (e.g. a completed-but-uninstalled download from a prior
    /// run). On mismatch the partial is deleted and
    /// [`StorageError::VerifyFailed`] is returned.
    pub fn verify_and_install(&self, sha256: &str) -> Result<PathBuf, StorageError> {
        let mut hasher = Sha256::new();
        // A full-length-partial verify always runs to completion: there is no
        // pause surface for it, so it never cancels.
        self.feed_partial(sha256, &mut HashWriter(&mut hasher), &|| false)?;
        let actual = hex_digest(&hasher.finalize());
        self.install_if_matches(sha256, &actual)
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

    /// Free bytes on the volume holding the store root, for the pre-download
    /// disk-space line. `None` means unknown; callers skip the warning.
    pub fn free_bytes(&self) -> Option<u64> {
        free_disk_bytes(&self.root)
    }

    /// Materializes a load-time symlink shim for a multi-part (split) GGUF model
    /// and returns the path to the first shard's symlink, which the engine loads
    /// as `model_path`. llama.cpp reconstructs the whole split from the sibling
    /// shards in that directory, so each symlink must keep the shard's ORIGINAL
    /// `<prefix>-NNNNN-of-MMMMM.gguf` name while pointing at its content-addressed
    /// blob.
    ///
    /// The shim lives under a directory keyed by the first shard's content sha
    /// (`root/shims/<first-sha>/`), an address Thuki owns: two repos whose first
    /// shard happens to share a filename can never collide, and re-loading the
    /// same model reuses the same directory.
    ///
    /// Security: every `part.file` comes from an untrusted Hugging Face listing
    /// and is validated through [`crate::models::parse_shard`] before it is used
    /// as a symlink leaf, rejecting path separators, `..`, and any non-shard
    /// shape. An invalid name fails the whole call rather than touching disk.
    ///
    /// Idempotent: creation is purely additive. An existing same-named symlink
    /// necessarily points at the same blob (the directory is content-addressed),
    /// so an `AlreadyExists` is ignored; every other I/O error propagates. This
    /// makes a concurrent re-materialize (warmup racing a chat) safe without any
    /// remove-then-recreate window that could strand a load mid-flight.
    ///
    /// # Errors
    ///
    /// Returns an error when `parts` is empty, when a shard name fails
    /// validation, or on any filesystem failure other than `AlreadyExists`.
    pub fn materialize_split_shim(&self, parts: &[HfGgufPart]) -> io::Result<PathBuf> {
        let first = parts.first().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot materialize a split shim for an empty shard set",
            )
        })?;
        let dir = self.root.join("shims").join(&first.sha256);
        std::fs::create_dir_all(&dir)?;

        for part in parts {
            // Reject an attacker-controlled shard name before it becomes a path.
            // `parse_shard` enforces the `<prefix>-NNNNN-of-MMMMM.gguf` shape but
            // not that the name is a single path component, so guard separators
            // and traversal explicitly: the name must be exactly one normal path
            // component (no `/`, no `..`, not absolute) AND a valid shard shape.
            if !is_safe_shard_leaf(&part.file) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("refusing to shim an invalid shard name: {}", part.file),
                ));
            }
            let link = dir.join(&part.file);
            match std::os::unix::fs::symlink(self.blob_path(&part.sha256), &link) {
                Ok(()) => {}
                // A pre-existing link points at the right blob (content-addressed
                // dir), so treat it as already materialized.
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
        }
        // The first shard's symlink is the engine's load entry point; it was
        // validated and created on the loop's first iteration.
        Ok(dir.join(&parts[0].file))
    }

    /// Removes the entire split-shim directory tree (`root/shims/`). A no-op when
    /// it does not exist, so callers need not pre-check. Called at app quit, after
    /// the engine has shut down and no load can be in flight; the underlying shard
    /// blobs survive in `root/blobs/` regardless, so removing the symlinks frees
    /// only a few bytes of indirection.
    pub fn clear_split_shims(&self) -> io::Result<()> {
        match std::fs::remove_dir_all(self.root.join("shims")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// True when `name` is safe to use as a split-shim symlink leaf: a single
/// normal path component (no separators, no `..`, not absolute, not empty or
/// `.`) that is also a well-formed `<prefix>-NNNNN-of-MMMMM.gguf` shard name.
///
/// The path-component check is the security boundary: it stops an
/// attacker-controlled Hugging Face filename like `../../etc/evil-00001-of-00001.gguf`
/// (which [`crate::models::parse_shard`] alone accepts, since it only inspects
/// the fixed suffix and a non-empty prefix) from escaping the shim directory.
fn is_safe_shard_leaf(name: &str) -> bool {
    use std::path::Component;
    let mut components = std::path::Path::new(name).components();
    matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) && crate::models::parse_shard(name).is_some()
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
    use std::io::Write as _;
    use tempfile::TempDir;

    #[test]
    fn hash_writer_forwards_writes_and_flush_is_a_no_op() {
        let mut hasher = Sha256::new();
        {
            let mut writer = HashWriter(&mut hasher);
            writer.write_all(b"abc").unwrap();
            writer.flush().unwrap();
        }
        assert_eq!(hex_digest(&hasher.finalize()), sha256_of(b"abc"));
    }

    /// Build a fresh store rooted at a temporary directory.
    fn make_store() -> (TempDir, ModelStore) {
        let dir = TempDir::new().unwrap();
        let store = ModelStore::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    /// Compute the hex SHA-256 of `data`.
    fn sha256_of(data: &[u8]) -> String {
        hex_digest(&Sha256::digest(data))
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

    // ── feed_partial cancellation ────────────────────────────────────────────

    #[test]
    fn feed_partial_reads_the_whole_partial_when_not_cancelled() {
        let (_dir, store) = make_store();
        let sha = "feeddone";
        let data = b"some bytes to stream through the sink";
        write_partial(&store, sha, data);

        let mut sink = Vec::new();
        store.feed_partial(sha, &mut sink, &|| false).unwrap();
        assert_eq!(sink, data);
    }

    #[test]
    fn feed_partial_stops_early_when_cancelled() {
        let (_dir, store) = make_store();
        let sha = "feedcancel";
        // Two full read buffers, so the cancel can land after the first.
        let data = vec![7u8; BLOB_HASH_BUFFER_BYTES * 2];
        write_partial(&store, sha, &data);

        let mut sink = Vec::new();
        let checks = std::cell::Cell::new(0u32);
        store
            .feed_partial(sha, &mut sink, &|| {
                let n = checks.get();
                checks.set(n + 1);
                // False on the first check (one buffer is read), true after.
                n >= 1
            })
            .unwrap();
        assert!(
            sink.len() < data.len(),
            "feed_partial must stop before reading the whole partial"
        );
    }

    // ── suspended hash (in-memory resume) ────────────────────────────────────

    #[test]
    fn suspended_hash_round_trips_and_continues() {
        let (_dir, store) = make_store();
        // A paused download whose running hash has consumed "abc".
        let mut hasher = Sha256::new();
        hasher.update(b"abc");
        store.save_suspended_hash("aa", 3, hasher);

        // Resuming takes it back and continues with the remaining bytes; the
        // result must equal hashing the whole stream in one pass.
        let mut taken = store.take_suspended_hash("aa", 3).unwrap();
        taken.update(b"def");
        assert_eq!(hex_digest(&taken.finalize()), sha256_of(b"abcdef"));
    }

    #[test]
    fn suspended_hash_take_clears_the_slot() {
        let (_dir, store) = make_store();
        store.save_suspended_hash("aa", 3, Sha256::new());
        assert!(store.take_suspended_hash("aa", 3).is_some());
        // The slot is now empty: a second take finds nothing.
        assert!(store.take_suspended_hash("aa", 3).is_none());
    }

    #[test]
    fn suspended_hash_is_dropped_on_a_mismatch() {
        let (_dir, store) = make_store();
        // A different sha clears the stale entry and returns None.
        store.save_suspended_hash("aa", 3, Sha256::new());
        assert!(store.take_suspended_hash("bb", 3).is_none());
        assert!(store.take_suspended_hash("aa", 3).is_none());
        // A length that no longer matches the on-disk partial returns None.
        store.save_suspended_hash("aa", 3, Sha256::new());
        assert!(store.take_suspended_hash("aa", 9).is_none());
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

    #[test]
    fn store_free_bytes_delegates_to_root_volume() {
        let (_dir, store) = make_store();
        let free = store.free_bytes();
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

    // ── split-shim materialization ───────────────────────────────────────────

    /// Build a part with a well-formed split name and a dummy blob written for
    /// its sha so the symlink resolves to real bytes.
    fn shard(store: &ModelStore, index: u32, total: u32, sha: &str) -> HfGgufPart {
        std::fs::write(store.blob_path(sha), format!("blob:{sha}")).unwrap();
        HfGgufPart {
            file: format!("model-{index:05}-of-{total:05}.gguf"),
            sha256: sha.to_string(),
            size_bytes: 100,
        }
    }

    #[test]
    fn materialize_split_shim_links_every_shard_to_its_blob() {
        let (_dir, store) = make_store();
        let parts = vec![shard(&store, 1, 2, "shaone"), shard(&store, 2, 2, "shatwo")];

        let first = store.materialize_split_shim(&parts).unwrap();
        // The returned path is the first shard's symlink, named by its original
        // split filename and living under the first shard's sha.
        assert!(first.ends_with("shims/shaone/model-00001-of-00002.gguf"));

        // Every symlink resolves to the matching blob's bytes.
        for part in &parts {
            let link = first.parent().unwrap().join(&part.file);
            assert_eq!(
                std::fs::read(&link).unwrap(),
                format!("blob:{}", part.sha256).into_bytes()
            );
        }
    }

    #[test]
    fn materialize_split_shim_is_idempotent() {
        let (_dir, store) = make_store();
        let parts = vec![shard(&store, 1, 2, "idemA"), shard(&store, 2, 2, "idemB")];

        let first = store.materialize_split_shim(&parts).unwrap();
        // A second call over the same parts must succeed (AlreadyExists ignored)
        // and return the same path.
        let again = store.materialize_split_shim(&parts).unwrap();
        assert_eq!(first, again);
        assert!(again.exists());
    }

    #[test]
    fn materialize_split_shim_rejects_an_empty_shard_set() {
        let (_dir, store) = make_store();
        let err = store.materialize_split_shim(&[]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn materialize_split_shim_rejects_malicious_shard_names() {
        let (_dir, store) = make_store();
        // Each is an attacker-controlled name that must be refused before any
        // symlink is created: traversal, a nested path, an absolute path, and a
        // name with the right shard suffix but path separators in the prefix.
        for evil in [
            "../escape.gguf",                     // not a shard shape at all
            "../../etc/evil-00001-of-00001.gguf", // traversal with shard suffix
            "sub/dir/model-00001-of-00001.gguf",  // nested path
            "/abs/model-00001-of-00001.gguf",     // absolute path
        ] {
            let parts = vec![HfGgufPart {
                file: evil.to_string(),
                sha256: "evilsha".to_string(),
                size_bytes: 100,
            }];
            let err = store.materialize_split_shim(&parts).unwrap_err();
            assert_eq!(
                err.kind(),
                io::ErrorKind::InvalidInput,
                "must reject malicious shard name: {evil}"
            );
        }
    }

    #[test]
    fn materialize_split_shim_propagates_a_non_already_exists_symlink_error() {
        let (_dir, store) = make_store();
        // A 300-char prefix passes is_safe_shard_leaf (single component, valid
        // shard shape: parse_shard does not bound prefix length) but the
        // filesystem rejects the ~320-char leaf with ENAMETOOLONG, exercising
        // the non-AlreadyExists symlink error arm.
        let long_name = format!("{}-00001-of-00001.gguf", "x".repeat(300));
        let parts = vec![HfGgufPart {
            file: long_name,
            sha256: "longsha".to_string(),
            size_bytes: 100,
        }];
        let err = store.materialize_split_shim(&parts).unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn clear_split_shims_propagates_a_non_not_found_error() {
        let (_dir, store) = make_store();
        // Place a regular FILE where the shims directory would be, so
        // remove_dir_all returns a non-NotFound error that must propagate.
        std::fs::write(store.root.join("shims"), b"not a dir").unwrap();
        let err = store.clear_split_shims().unwrap_err();
        assert_ne!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn clear_split_shims_removes_the_tree_and_is_idempotent() {
        let (_dir, store) = make_store();
        // No shims dir yet: clearing is a no-op.
        store.clear_split_shims().unwrap();

        // A valid single-shard (1-of-1) set so a shim tree exists to remove.
        let parts = vec![shard(&store, 1, 1, "clr1")];
        let link = store.materialize_split_shim(&parts).unwrap();
        assert!(link.exists());

        store.clear_split_shims().unwrap();
        assert!(!store.root.join("shims").exists());
        // Clearing again with the tree already gone is still a no-op.
        store.clear_split_shims().unwrap();
    }
}
