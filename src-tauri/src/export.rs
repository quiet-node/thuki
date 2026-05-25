/*!
 * Chat session export.
 *
 * The frontend serialises the active conversation to a Markdown or plain
 * text string and asks this module to persist it. The native save dialog
 * AND the write both live on the Rust side so the destination path is
 * never an attacker-influenceable IPC argument: the renderer hands over
 * only the serialised content, the suggested filename, and the requested
 * format. The path returned by the dialog stays inside this module and
 * is consumed by [`write_export`] without round-tripping through JS.
 *
 * This closes the trust gap that a separate "open save dialog" command
 * plus "write to path the renderer chose" command would leave open: a
 * compromised renderer could otherwise drive the write at any path the
 * app process can reach. With dialog and write fused, the path comes
 * from `NSSavePanel` exclusively.
 */

use std::fs;
use std::path::{Path, PathBuf};

/// Format chosen in the export popover. Determines the primary filter in
/// the save dialog and the default-filename extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    /// Markdown body (with YAML frontmatter); typically `.md`.
    Markdown,
    /// Plain text body; typically `.txt`.
    PlainText,
}

impl ExportFormat {
    /// Parses the string sent by the frontend. Anything other than the
    /// two known tokens is treated as Markdown so a frontend regression
    /// degrades to the safer default rather than rejecting the export.
    pub fn parse(value: &str) -> Self {
        match value {
            "txt" => ExportFormat::PlainText,
            _ => ExportFormat::Markdown,
        }
    }
}

/// Save-dialog filter spec. Kept as plain data so the construction logic
/// is unit-testable without spinning up Tauri or AppKit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogFilter {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
}

/// Returns the dialog filter list for the requested format. The chosen
/// format becomes the PRIMARY filter (top of the macOS dropdown) so the
/// dialog opens with the matching extension pre-selected. The other
/// format remains available as the second entry so the user can still
/// switch without re-opening the popover.
pub fn build_save_filters(format: ExportFormat) -> Vec<DialogFilter> {
    let markdown = DialogFilter {
        name: "Markdown",
        extensions: &["md"],
    };
    let plain_text = DialogFilter {
        name: "Plain text",
        extensions: &["txt"],
    };
    match format {
        ExportFormat::Markdown => vec![markdown, plain_text],
        ExportFormat::PlainText => vec![plain_text, markdown],
    }
}

/// Failure modes for [`write_export`]. Carries no path strings: the
/// IPC-facing error message never leaks the destination the user
/// picked, which would otherwise surface in screenshots and screen
/// recordings.
#[derive(Debug)]
pub enum ExportError {
    /// Path was empty after trimming. Treated as a cancellation-shaped
    /// failure rather than something worth surfacing in detail.
    EmptyPath,
    /// `std::fs::write` failed. The variant captures only the OS-level
    /// error kind; the user-facing message is a fixed string per kind
    /// so absolute paths never appear in the banner.
    Write(std::io::ErrorKind),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::EmptyPath => write!(f, "Export path is empty"),
            ExportError::Write(kind) => write!(f, "{}", write_error_message(*kind)),
        }
    }
}

/// User-facing message for an `io::Error` kind. Kept short and concrete
/// so the banner reads as actionable rather than raw OS jargon, and
/// devoid of any filesystem path the user chose.
pub fn write_error_message(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::PermissionDenied => "Permission denied. Choose a writable location.",
        std::io::ErrorKind::NotFound => "The selected location does not exist.",
        std::io::ErrorKind::AlreadyExists => "A file already exists at that location.",
        std::io::ErrorKind::InvalidInput => "The selected filename is invalid.",
        std::io::ErrorKind::OutOfMemory => "Out of memory while writing the export.",
        std::io::ErrorKind::StorageFull => "The disk is full.",
        std::io::ErrorKind::ReadOnlyFilesystem => "The selected location is read-only.",
        _ => "Failed to write the export.",
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
    write_export_path(&target, content)?;
    Ok(target)
}

/// Internal write that takes an already-resolved `Path`. Split out so
/// the dialog-driven command path can hand a `Path` straight in without
/// re-serialising to a string just to satisfy the trim guard above.
fn write_export_path(path: &Path, content: &str) -> Result<(), ExportError> {
    fs::write(path, content).map_err(|e| ExportError::Write(e.kind()))
}

/// Tauri command: opens the native save dialog with the appropriate
/// filters for the requested format, then writes `content` to whichever
/// path the user picked. Returns `true` if a file was written, `false`
/// if the user cancelled the dialog, and `Err(message)` on a write
/// failure. The destination path is consumed entirely inside Rust and
/// never crosses the IPC boundary.
#[cfg(not(coverage))]
#[tauri::command]
#[cfg_attr(coverage_nightly, coverage(off))]
pub async fn prompt_and_save_chat_export<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    content: String,
    default_filename: String,
    format: String,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    use tokio::sync::oneshot;

    let parsed = ExportFormat::parse(&format);
    let filters = build_save_filters(parsed);

    let mut builder = app.dialog().file().set_file_name(&default_filename);
    for filter in &filters {
        builder = builder.add_filter(filter.name, filter.extensions);
    }

    let (tx, rx) = oneshot::channel();
    builder.save_file(move |maybe_path| {
        let _ = tx.send(maybe_path);
    });

    let maybe_path = rx
        .await
        .map_err(|_| "save dialog channel closed unexpectedly".to_string())?;
    let Some(file_path) = maybe_path else {
        return Ok(false);
    };
    let path: PathBuf = file_path.into_path().map_err(|e| e.to_string())?;
    write_export_path(&path, &content).map_err(|e| e.to_string())?;
    Ok(true)
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
    fn write_error_display_never_leaks_path() {
        let err = ExportError::Write(std::io::ErrorKind::PermissionDenied);
        let msg = format!("{err}");
        assert_eq!(msg, "Permission denied. Choose a writable location.");
        assert!(
            !msg.contains('/'),
            "user-facing message must not include a filesystem path"
        );
    }

    #[test]
    fn write_error_messages_cover_known_kinds() {
        assert_eq!(
            write_error_message(std::io::ErrorKind::PermissionDenied),
            "Permission denied. Choose a writable location."
        );
        assert_eq!(
            write_error_message(std::io::ErrorKind::NotFound),
            "The selected location does not exist."
        );
        assert_eq!(
            write_error_message(std::io::ErrorKind::AlreadyExists),
            "A file already exists at that location."
        );
        assert_eq!(
            write_error_message(std::io::ErrorKind::InvalidInput),
            "The selected filename is invalid."
        );
        assert_eq!(
            write_error_message(std::io::ErrorKind::OutOfMemory),
            "Out of memory while writing the export."
        );
        assert_eq!(
            write_error_message(std::io::ErrorKind::StorageFull),
            "The disk is full."
        );
        assert_eq!(
            write_error_message(std::io::ErrorKind::ReadOnlyFilesystem),
            "The selected location is read-only."
        );
        assert_eq!(
            write_error_message(std::io::ErrorKind::Other),
            "Failed to write the export."
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

    #[test]
    fn format_parse_recognises_known_tokens() {
        assert_eq!(ExportFormat::parse("md"), ExportFormat::Markdown);
        assert_eq!(ExportFormat::parse("txt"), ExportFormat::PlainText);
    }

    #[test]
    fn format_parse_unknown_defaults_to_markdown() {
        assert_eq!(ExportFormat::parse(""), ExportFormat::Markdown);
        assert_eq!(ExportFormat::parse("pdf"), ExportFormat::Markdown);
        assert_eq!(ExportFormat::parse("MD"), ExportFormat::Markdown);
    }

    #[test]
    fn save_filters_markdown_first() {
        let filters = build_save_filters(ExportFormat::Markdown);
        assert_eq!(
            filters,
            vec![
                DialogFilter {
                    name: "Markdown",
                    extensions: &["md"]
                },
                DialogFilter {
                    name: "Plain text",
                    extensions: &["txt"]
                },
            ]
        );
    }

    #[test]
    fn save_filters_plain_text_first() {
        let filters = build_save_filters(ExportFormat::PlainText);
        assert_eq!(
            filters,
            vec![
                DialogFilter {
                    name: "Plain text",
                    extensions: &["txt"]
                },
                DialogFilter {
                    name: "Markdown",
                    extensions: &["md"]
                },
            ]
        );
    }
}
