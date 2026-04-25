/*!
 * Image storage and lifecycle management.
 *
 * Images are stored in a flat directory at `<app_data_dir>/images/` with
 * UUID-based filenames. This follows the industry-standard pattern used by
 * Signal, iMessage, and Slack - media files are independent entities linked
 * to messages through path references, not organized by conversation.
 *
 * Each image is compressed to JPEG (quality 85, max 1920px) on save to keep
 * disk usage and Ollama inference latency low.
 *
 * Lifecycle:
 * - **Paste/drop:** frontend sends raw bytes → `save_image` compresses and
 *   writes to the flat images directory, returns the file path.
 * - **Remove:** user clicks "X" on a thumbnail → `remove_image` deletes the
 *   file from disk.
 * - **Cleanup:** `cleanup_orphaned_images` removes files not referenced by
 *   any saved message. Runs on startup and periodically.
 */

use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use image::imageops::FilterType;
use tauri::Manager;

/// Maximum dimension (width or height) for saved images. Images exceeding this
/// are downscaled proportionally, preserving aspect ratio.
const MAX_DIMENSION: u32 = 1920;

/// JPEG compression quality (1–100). 85 balances file size and visual fidelity
/// for vision model consumption.
const JPEG_QUALITY: u8 = 85;

/// Maximum number of images allowed per message (3 manual + 1 /screen = 4).
pub const MAX_IMAGES_PER_MESSAGE: usize = 4;

/// Resolves the root images directory: `<base_dir>/images/`.
pub fn images_root(base_dir: &Path) -> PathBuf {
    base_dir.join("images")
}

/// Returns a closure that formats an error with a contextual message prefix.
/// One shared generic instantiation instead of N separate closure functions
/// in the llvm-cov function table.
fn err<E: std::fmt::Display>(context: &'static str) -> impl FnOnce(E) -> String {
    move |e| format!("{context}: {e}")
}

/// Compresses raw image bytes to JPEG (max 1920px) and writes to the flat
/// images directory with a UUID filename.
///
/// Returns the absolute path of the saved file. The caller owns the path and
/// can pass it to the frontend for `asset://` rendering.
///
/// # Errors
///
/// Returns an error if the image bytes cannot be decoded, the output directory
/// cannot be created, or the file cannot be written.
pub fn save_image(base_dir: &Path, image_data: &[u8]) -> Result<String, String> {
    let img = image::load_from_memory(image_data).map_err(err("failed to decode image"))?;

    let resized = if img.width() > MAX_DIMENSION || img.height() > MAX_DIMENSION {
        img.resize(MAX_DIMENSION, MAX_DIMENSION, FilterType::Lanczos3)
    } else {
        img
    };

    let dir = images_root(base_dir);
    std::fs::create_dir_all(&dir).map_err(err("failed to create image directory"))?;

    let filename = format!("{}.jpg", uuid::Uuid::new_v4());
    let path = dir.join(&filename);

    let rgb = resized.to_rgb8();
    let mut jpeg_buf = Vec::new();
    {
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, JPEG_QUALITY);
        encoder
            .encode_image(&rgb)
            .map_err(err("failed to encode JPEG"))?;
    }

    std::fs::write(&path, &jpeg_buf).map_err(err("failed to write image file"))?;

    path.to_str()
        .map(|s| s.to_string())
        .ok_or("image path contains non-UTF-8 characters".to_string())
}

/// Deletes a single image file from disk, provided it resides within the
/// given `base_dir/images/` directory. Rejects paths outside the images root
/// to prevent path-traversal attacks via the IPC boundary.
///
/// # Errors
///
/// Returns an error if the path escapes the images directory or the file
/// cannot be removed. Silently succeeds if the file does not exist (idempotent).
pub fn remove_image(base_dir: &Path, path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(());
    }
    let canonical = p
        .canonicalize()
        .map_err(err("failed to resolve image path"))?;
    let root = images_root(base_dir)
        .canonicalize()
        .map_err(err("failed to resolve images root"))?;
    if !canonical.starts_with(&root) {
        return Err("path is outside the images directory".to_string());
    }
    std::fs::remove_file(p).map_err(err("failed to remove image"))?;
    Ok(())
}

/// Removes image files that are not in the set of referenced paths.
///
/// `referenced_paths` contains the absolute paths of all images currently
/// referenced by saved messages. Any file in `<base_dir>/images/` not in
/// this set is deleted.
///
/// # Errors
///
/// Returns an error if the images root directory cannot be read.
pub fn cleanup_orphaned_images(
    base_dir: &Path,
    referenced_paths: &[String],
) -> Result<usize, String> {
    let root = images_root(base_dir);
    if !root.exists() {
        return Ok(0);
    }

    let entries = std::fs::read_dir(&root).map_err(err("failed to read images directory"))?;

    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let path_str = path.to_string_lossy().to_string();
        if !referenced_paths.contains(&path_str) && std::fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    Ok(removed)
}

/// Reads image files from disk and returns their base64-encoded contents
/// for inclusion in Ollama API requests.
///
/// # Errors
///
/// Returns an error if any file cannot be read.
pub fn encode_images_as_base64(paths: &[String]) -> Result<Vec<String>, String> {
    paths
        .iter()
        .map(|p| {
            let bytes = std::fs::read(p).map_err(|e| format!("failed to read image {p}: {e}"))?;
            Ok(BASE64.encode(&bytes))
        })
        .collect()
}

// ─── Tauri commands ────────────────────────────────────────────────────────
//
// Thin wrappers that delegate to the pure functions above. Excluded from
// coverage builds entirely (`#[cfg(not(coverage))]`) because `coverage(off)`
// suppresses instrumentation but llvm-cov still counts excluded function
// signatures as "missed lines" in the summary - breaking the 100% gate.

/// Compresses and saves an image to the flat images directory.
///
/// Accepts base64-encoded image data as a string to avoid the performance
/// penalty of JSON-serializing a `Vec<u8>` (millions of individual numbers)
/// over the Tauri IPC bridge.
///
/// The command is `async` so Tauri runs it off the main thread. The heavy
/// work (base64 decode → image decode → Lanczos3 resize → JPEG encode) is
/// dispatched to `spawn_blocking` to avoid blocking the async runtime,
/// keeping the WebView UI fully responsive during processing.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn save_image_command(
    app_handle: tauri::AppHandle,
    image_data_base64: String,
) -> Result<String, String> {
    let base_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;

    tokio::task::spawn_blocking(move || {
        let image_data = BASE64
            .decode(&image_data_base64)
            .map_err(|e| format!("failed to decode base64: {e}"))?;
        save_image(&base_dir, &image_data)
    })
    .await
    .map_err(|e| format!("image processing task failed: {e}"))?
}

/// Deletes a single image file from disk (with path containment check).
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn remove_image_command(app_handle: tauri::AppHandle, path: String) -> Result<(), String> {
    let base_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    remove_image(&base_dir, &path)
}

/// Removes image files not referenced by any saved message.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn cleanup_orphaned_images_command(
    app_handle: tauri::AppHandle,
    referenced_paths: Vec<String>,
) -> Result<usize, String> {
    let base_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    cleanup_orphaned_images(&base_dir, &referenced_paths)
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Creates a minimal valid 1x1 red PNG for testing.
    fn tiny_png() -> Vec<u8> {
        let mut buf = Vec::new();
        let img = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]));
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let mut cursor = std::io::Cursor::new(&mut buf);
        dyn_img
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        buf
    }

    /// Creates a large PNG (2000x1500) that exceeds MAX_DIMENSION.
    fn large_png() -> Vec<u8> {
        let mut buf = Vec::new();
        let img = image::RgbImage::from_pixel(2000, 1500, image::Rgb([0, 128, 255]));
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let mut cursor = std::io::Cursor::new(&mut buf);
        dyn_img
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();
        buf
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("thuki-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_image_creates_jpeg_file() {
        let base = temp_dir();
        let path = save_image(&base, &tiny_png()).unwrap();

        assert!(Path::new(&path).exists());
        assert!(path.ends_with(".jpg"));
        // File should be in the flat images/ directory, not a subdirectory.
        assert!(path.contains("/images/"));

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_image_compresses_large_image() {
        let base = temp_dir();
        let path = save_image(&base, &large_png()).unwrap();

        let saved = image::open(&path).unwrap();
        assert!(saved.width() <= MAX_DIMENSION);
        assert!(saved.height() <= MAX_DIMENSION);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_image_rejects_invalid_bytes() {
        let base = temp_dir();
        let result = save_image(&base, b"not an image");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to decode image"));

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_image_preserves_aspect_ratio() {
        let base = temp_dir();
        let mut buf = Vec::new();
        let img = image::RgbImage::from_pixel(3000, 1000, image::Rgb([0, 0, 0]));
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let mut cursor = std::io::Cursor::new(&mut buf);
        dyn_img
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();

        let path = save_image(&base, &buf).unwrap();
        let saved = image::open(&path).unwrap();

        assert_eq!(saved.width(), MAX_DIMENSION);
        assert!(saved.height() < MAX_DIMENSION);
        assert_eq!(saved.height(), 640);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_image_does_not_upscale_small_images() {
        let base = temp_dir();
        let path = save_image(&base, &tiny_png()).unwrap();
        let saved = image::open(&path).unwrap();

        assert_eq!(saved.width(), 1);
        assert_eq!(saved.height(), 1);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn remove_image_deletes_file() {
        let base = temp_dir();
        let path = save_image(&base, &tiny_png()).unwrap();
        assert!(Path::new(&path).exists());

        remove_image(&base, &path).unwrap();
        assert!(!Path::new(&path).exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn remove_image_idempotent_on_missing_file() {
        let base = temp_dir();
        let result = remove_image(&base, "/tmp/nonexistent-thuki-image.jpg");
        assert!(result.is_ok());
        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn remove_image_rejects_path_outside_images_dir() {
        let base = temp_dir();
        fs::create_dir_all(images_root(&base)).unwrap();
        let outside = base.join("secret.txt");
        fs::write(&outside, b"sensitive").unwrap();

        let result = remove_image(&base, outside.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("outside the images directory"));
        // File must still exist - not deleted.
        assert!(outside.exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn cleanup_orphaned_images_removes_unreferenced_files() {
        let base = temp_dir();
        let kept = save_image(&base, &tiny_png()).unwrap();
        let orphan = save_image(&base, &tiny_png()).unwrap();

        let referenced = vec![kept.clone()];
        let removed = cleanup_orphaned_images(&base, &referenced).unwrap();

        assert_eq!(removed, 1);
        assert!(Path::new(&kept).exists());
        assert!(!Path::new(&orphan).exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn cleanup_orphaned_images_noop_when_no_images_dir() {
        let base = temp_dir();
        let removed = cleanup_orphaned_images(&base, &[]).unwrap();
        assert_eq!(removed, 0);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn cleanup_orphaned_images_removes_all_when_no_references() {
        let base = temp_dir();
        save_image(&base, &tiny_png()).unwrap();
        save_image(&base, &tiny_png()).unwrap();

        let removed = cleanup_orphaned_images(&base, &[]).unwrap();
        assert_eq!(removed, 2);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn cleanup_orphaned_images_preserves_all_when_all_referenced() {
        let base = temp_dir();
        let p1 = save_image(&base, &tiny_png()).unwrap();
        let p2 = save_image(&base, &tiny_png()).unwrap();

        let referenced = vec![p1, p2];
        let removed = cleanup_orphaned_images(&base, &referenced).unwrap();
        assert_eq!(removed, 0);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn cleanup_orphaned_images_skips_subdirectories() {
        let base = temp_dir();
        save_image(&base, &tiny_png()).unwrap();
        // Create a subdirectory that should be skipped (not a file).
        fs::create_dir_all(images_root(&base).join("stray-dir")).unwrap();

        let removed = cleanup_orphaned_images(&base, &[]).unwrap();
        // Only the file should be removed, not the directory.
        assert_eq!(removed, 1);
        assert!(images_root(&base).join("stray-dir").exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn encode_images_as_base64_roundtrip() {
        let base = temp_dir();
        let path = save_image(&base, &tiny_png()).unwrap();

        let encoded = encode_images_as_base64(std::slice::from_ref(&path)).unwrap();
        assert_eq!(encoded.len(), 1);

        let decoded = BASE64.decode(&encoded[0]).unwrap();
        assert!(!decoded.is_empty());
        // JPEG magic bytes: FF D8.
        assert_eq!(decoded[0], 0xFF);
        assert_eq!(decoded[1], 0xD8);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn encode_images_as_base64_empty_list() {
        let result = encode_images_as_base64(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn encode_images_as_base64_rejects_missing_file() {
        let result = encode_images_as_base64(&["/tmp/nonexistent-thuki.jpg".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn images_root_resolves_correctly() {
        let base = Path::new("/tmp/thuki-test");
        assert_eq!(images_root(base), PathBuf::from("/tmp/thuki-test/images"));
    }

    #[test]
    fn max_images_per_message_is_four() {
        assert_eq!(MAX_IMAGES_PER_MESSAGE, 4);
    }

    #[test]
    fn err_helper_formats_context_and_cause() {
        let format_fn = err("failed to frobnicate");
        assert_eq!(format_fn("disk full"), "failed to frobnicate: disk full");
    }
}
