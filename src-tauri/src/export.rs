/*!
 * Chat session export.
 *
 * The frontend serialises the active conversation to a Markdown string
 * (frontmatter, role-labelled blocks, inline base64 images via the Tauri
 * asset protocol). This module is the trust boundary at which the chosen
 * destination path becomes a real write.
 *
 * The native save dialog is the user's explicit consent: a path returned
 * by it is, by construction, where the user wants the file. We do not
 * second-guess directory traversal or path scope here. We do reject
 * an empty / whitespace-only path because Tauri's `dialog::save` returns
 * an opaque `String` that the frontend may relay verbatim, and an empty
 * string would otherwise resolve to the current working directory on
 * `std::fs::write` and silently overwrite something.
 */

use std::fs;
use std::path::PathBuf;

/// Failure modes for [`write_export`]. Mapped to plain strings before
/// crossing the IPC boundary.
#[derive(Debug)]
pub enum ExportError {
    /// Path was empty after trimming. Treated as user cancellation rather
    /// than an error worth surfacing in detail.
    EmptyPath,
    /// `std::fs::write` failed. The wrapped `io::Error` includes the
    /// OS-level reason (permission denied, no such directory, etc.).
    Write(std::io::Error),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::EmptyPath => write!(f, "Export path is empty"),
            ExportError::Write(e) => write!(f, "{e}"),
        }
    }
}

/// Writes `content` to `path`, returning the resolved [`PathBuf`].
///
/// Trims `path` to be lenient about trailing whitespace from the dialog
/// (some macOS save sheets occasionally append a trailing newline when
/// the user typed into the filename field). An empty or whitespace-only
/// path is rejected so the file is never written to the process working
/// directory by accident.
pub fn write_export(path: &str, content: &str) -> Result<PathBuf, ExportError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ExportError::EmptyPath);
    }
    let target = PathBuf::from(trimmed);
    fs::write(&target, content).map_err(ExportError::Write)?;
    Ok(target)
}

/// Tauri command: persists a serialised chat-session Markdown document
/// to the path the user chose in the native save dialog.
///
/// Thin wrapper over [`write_export`]; covered by the unit tests on
/// `write_export`, which is what the wrapper delegates to.
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn save_chat_export(path: String, content: String) -> Result<(), String> {
    write_export(&path, &content)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn empty_path_is_rejected() {
        let err = write_export("", "hello").expect_err("empty path must error");
        assert!(matches!(err, ExportError::EmptyPath));
    }

    #[test]
    fn whitespace_only_path_is_rejected() {
        let err = write_export("   \t\n", "hello").expect_err("whitespace must error");
        assert!(matches!(err, ExportError::EmptyPath));
    }

    #[test]
    fn empty_path_display_is_user_facing() {
        assert_eq!(
            format!("{}", ExportError::EmptyPath),
            "Export path is empty"
        );
    }

    #[test]
    fn write_error_display_forwards_io_message() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let msg = format!("{}", ExportError::Write(io));
        assert!(
            msg.contains("denied"),
            "io message must propagate, got: {msg}"
        );
    }

    #[test]
    fn valid_path_writes_content_and_returns_path() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("export.md");
        let target_str = target.to_str().expect("utf-8");

        let returned = write_export(target_str, "# Hello\n\nWorld").expect("write must succeed");

        assert_eq!(returned, target);
        let read_back = fs::read_to_string(&target).expect("file must exist");
        assert_eq!(read_back, "# Hello\n\nWorld");
    }

    #[test]
    fn trailing_whitespace_in_path_is_trimmed() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("trimmed.md");
        let padded = format!("  {}  \n", target.to_str().expect("utf-8"));

        let returned = write_export(&padded, "content").expect("write must succeed");

        assert_eq!(returned, target);
        assert!(
            target.exists(),
            "file should be written to the trimmed path"
        );
    }

    #[test]
    fn nonexistent_directory_returns_write_error() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("does/not/exist/export.md");
        let target_str = target.to_str().expect("utf-8");

        let err = write_export(target_str, "x").expect_err("write must fail");
        assert!(matches!(err, ExportError::Write(_)));
    }

    #[test]
    fn overwrites_existing_file() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("rewrite.md");
        fs::write(&target, "old").expect("seed");

        write_export(target.to_str().expect("utf-8"), "new").expect("overwrite");
        let read_back = fs::read_to_string(&target).expect("file must exist");
        assert_eq!(read_back, "new");
    }

    #[test]
    fn empty_content_writes_empty_file() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("empty.md");
        write_export(target.to_str().expect("utf-8"), "").expect("empty write");
        let read_back = fs::read_to_string(&target).expect("file must exist");
        assert_eq!(read_back, "");
    }
}
