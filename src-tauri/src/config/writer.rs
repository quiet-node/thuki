//! Atomic writer for the config file.
//!
//! Semantics: serialize `AppConfig` to TOML, write to a temporary file in the
//! same directory, fsync the tmpfile, rename over the target, fsync the
//! parent directory. Rename is atomic on HFS+/APFS, so a crash or power-loss
//! leaves either the old file intact (if rename has not happened) or the new
//! file intact (if rename has). No torn writes.
//!
//! v1 uses this only for the first-run default seed. The future settings-panel
//! PR will also call it from the `set_config` command, at which point the
//! lock-held-during-fsync trade-off documented in the design doc applies.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::schema::AppConfig;

/// Atomically writes `config` to `path`.
///
/// Returns the underlying `std::io::Error` on any I/O failure. The loader wraps
/// this into `ConfigError::SeedFailed` when the context is first-run seeding
/// (the only fatal path).
///
/// Parent-directory fsync after rename is best-effort; failures are silently
/// ignored because the data itself is already on disk via the prior file fsync.
pub fn atomic_write(path: &Path, config: &AppConfig) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "config path has no parent directory",
        )
    })?;

    std::fs::create_dir_all(parent)?;

    // AppConfig only contains simple scalars, strings, and vectors of strings,
    // all of which serialize cleanly. toml::to_string_pretty on this shape
    // cannot fail; if it ever does, that is a genuine bug and we want to know.
    let serialized =
        toml::to_string_pretty(config).expect("AppConfig is always serializable to TOML");

    let tmp_path = tmp_path_for(path);
    write_and_sync(&tmp_path, serialized.as_bytes())?;
    std::fs::rename(&tmp_path, path)?;

    // Best-effort parent fsync so the rename is durable across power loss.
    // Any failure here is dropped: the data itself has already been fsynced
    // to disk above, and some filesystems do not meaningfully support this.
    let _ = fsync_dir(parent);
    Ok(())
}

/// Returns a per-process, per-call temporary path in the same directory as
/// `target`. Using nanoseconds + process ID avoids collisions with other
/// concurrent writes even though v1 has only one writer.
fn tmp_path_for(target: &Path) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let mut s = target.as_os_str().to_os_string();
    s.push(format!(".tmp-{pid}-{nanos}"));
    s.into()
}

/// Writes `bytes` to `path` with mode 0600 (per-user read/write, nobody else),
/// then fsyncs the file before returning. On non-Unix this falls back to
/// default mode; Thuki is macOS-only so the permission bits always apply.
fn write_and_sync(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn fsync_dir(dir: &Path) -> std::io::Result<()> {
    let f = File::open(dir)?;
    f.sync_all()
}

#[cfg(not(unix))]
#[cfg_attr(coverage_nightly, coverage(off))]
fn fsync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}
