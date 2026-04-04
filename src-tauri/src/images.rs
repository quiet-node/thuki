/*!
 * Image storage and lifecycle management.
 *
 * Images are stored on disk under `<app_data_dir>/images/<conversation_id>/`.
 * Each image is compressed to JPEG (quality 85, max 1080p) on save to keep
 * disk usage and Ollama inference latency low.
 *
 * Lifecycle:
 * - **Paste/drop:** frontend sends raw bytes → `save_image` compresses and
 *   writes to the conversation's image directory, returns the file path.
 * - **Remove:** user clicks "X" on a thumbnail → `remove_image` deletes the
 *   file from disk.
 * - **Cleanup:** `cleanup_orphaned_images` removes directories not referenced
 *   by any saved conversation. Runs on startup and periodically.
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

/// Maximum number of images allowed per message.
pub const MAX_IMAGES_PER_MESSAGE: usize = 3;

/// Resolves the root images directory: `<base_dir>/images/`.
pub fn images_root(base_dir: &Path) -> PathBuf {
    base_dir.join("images")
}

/// Resolves the image directory for a specific conversation.
fn conversation_dir(base_dir: &Path, conversation_id: &str) -> PathBuf {
    images_root(base_dir).join(conversation_id)
}

/// Compresses raw image bytes to JPEG (max 1080p) and writes to disk.
///
/// Returns the absolute path of the saved file. The caller owns the path and
/// can pass it to the frontend for `asset://` rendering.
///
/// # Errors
///
/// Returns an error if the image bytes cannot be decoded, the output directory
/// cannot be created, or the file cannot be written.
pub fn save_image(
    base_dir: &Path,
    conversation_id: &str,
    image_data: &[u8],
) -> Result<String, String> {
    let img =
        image::load_from_memory(image_data).map_err(|e| format!("failed to decode image: {e}"))?;

    let resized = if img.width() > MAX_DIMENSION || img.height() > MAX_DIMENSION {
        img.resize(MAX_DIMENSION, MAX_DIMENSION, FilterType::Lanczos3)
    } else {
        img
    };

    let dir = conversation_dir(base_dir, conversation_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("failed to create image directory: {e}"))?;

    let filename = format!("{}.jpg", uuid::Uuid::new_v4());
    let path = dir.join(&filename);

    let rgb = resized.to_rgb8();
    let mut jpeg_buf = Vec::new();
    {
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, JPEG_QUALITY);
        encoder
            .encode_image(&rgb)
            .map_err(|e| format!("failed to encode JPEG: {e}"))?;
    }

    std::fs::write(&path, &jpeg_buf).map_err(|e| format!("failed to write image file: {e}"))?;

    path.to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "image path contains non-UTF-8 characters".to_string())
}

/// Deletes a single image file from disk.
///
/// # Errors
///
/// Returns an error if the file cannot be removed. Silently succeeds if the
/// file does not exist (idempotent).
pub fn remove_image(path: &str) -> Result<(), String> {
    let p = Path::new(path);
    if p.exists() {
        std::fs::remove_file(p).map_err(|e| format!("failed to remove image: {e}"))?;

        // Remove the parent directory if it is now empty.
        if let Some(parent) = p.parent() {
            if parent
                .read_dir()
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
            {
                let _ = std::fs::remove_dir(parent);
            }
        }
    }
    Ok(())
}

/// Removes image directories that are not referenced by any saved conversation.
///
/// `saved_ids` is the set of conversation IDs that currently exist in the
/// database. Any directory under `<base_dir>/images/` whose name is not in
/// this set is deleted.
///
/// # Errors
///
/// Returns an error if the images root directory cannot be read. Individual
/// directory removal failures are logged but do not fail the operation.
pub fn cleanup_orphaned_images(base_dir: &Path, saved_ids: &[String]) -> Result<usize, String> {
    let root = images_root(base_dir);
    if !root.exists() {
        return Ok(0);
    }

    let entries =
        std::fs::read_dir(&root).map_err(|e| format!("failed to read images directory: {e}"))?;

    let mut removed = 0;
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        if !saved_ids.contains(&dir_name) && std::fs::remove_dir_all(entry.path()).is_ok() {
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

/// Compresses and saves an image to the conversation's image directory.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn save_image_command(
    app_handle: tauri::AppHandle,
    conversation_id: String,
    image_data: Vec<u8>,
) -> Result<String, String> {
    let base_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    save_image(&base_dir, &conversation_id, &image_data)
}

/// Deletes a single image file from disk.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn remove_image_command(path: String) -> Result<(), String> {
    remove_image(&path)
}

/// Removes image directories not referenced by any saved conversation.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn cleanup_orphaned_images_command(
    app_handle: tauri::AppHandle,
    saved_ids: Vec<String>,
) -> Result<usize, String> {
    let base_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    cleanup_orphaned_images(&base_dir, &saved_ids)
}

/// Removes the entire image directory for a conversation.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn remove_conversation_images(
    app_handle: tauri::AppHandle,
    conversation_id: String,
) -> Result<(), String> {
    let base_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("failed to resolve app data dir: {e}"))?;
    let dir = conversation_dir(&base_dir, &conversation_id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("failed to remove conversation images: {e}"))?;
    }
    Ok(())
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
        let path = save_image(&base, "conv-1", &tiny_png()).unwrap();

        assert!(Path::new(&path).exists());
        assert!(path.ends_with(".jpg"));
        assert!(path.contains("conv-1"));

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_image_compresses_large_image() {
        let base = temp_dir();
        let path = save_image(&base, "conv-2", &large_png()).unwrap();

        // Verify the saved image was resized.
        let saved = image::open(&path).unwrap();
        assert!(saved.width() <= MAX_DIMENSION);
        assert!(saved.height() <= MAX_DIMENSION);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_image_rejects_invalid_bytes() {
        let base = temp_dir();
        let result = save_image(&base, "conv-3", b"not an image");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("failed to decode image"));

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_image_preserves_aspect_ratio() {
        let base = temp_dir();
        // Create a wide image: 3000x1000.
        let mut buf = Vec::new();
        let img = image::RgbImage::from_pixel(3000, 1000, image::Rgb([0, 0, 0]));
        let dyn_img = image::DynamicImage::ImageRgb8(img);
        let mut cursor = std::io::Cursor::new(&mut buf);
        dyn_img
            .write_to(&mut cursor, image::ImageFormat::Png)
            .unwrap();

        let path = save_image(&base, "conv-aspect", &buf).unwrap();
        let saved = image::open(&path).unwrap();

        // Width should be clamped to MAX_DIMENSION, height scaled proportionally.
        assert_eq!(saved.width(), MAX_DIMENSION);
        assert!(saved.height() < MAX_DIMENSION);
        // Aspect ratio: 3000/1000 = 3, so height ≈ 1920/3 = 640.
        assert_eq!(saved.height(), 640);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_image_does_not_upscale_small_images() {
        let base = temp_dir();
        let path = save_image(&base, "conv-small", &tiny_png()).unwrap();
        let saved = image::open(&path).unwrap();

        // 1x1 image should remain 1x1 (no upscaling).
        assert_eq!(saved.width(), 1);
        assert_eq!(saved.height(), 1);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn remove_image_deletes_file() {
        let base = temp_dir();
        let path = save_image(&base, "conv-4", &tiny_png()).unwrap();
        assert!(Path::new(&path).exists());

        remove_image(&path).unwrap();
        assert!(!Path::new(&path).exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn remove_image_cleans_up_empty_parent_dir() {
        let base = temp_dir();
        let path = save_image(&base, "conv-cleanup", &tiny_png()).unwrap();
        let parent = Path::new(&path).parent().unwrap().to_path_buf();

        remove_image(&path).unwrap();
        // Parent directory should be removed since it's now empty.
        assert!(!parent.exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn remove_image_idempotent_on_missing_file() {
        let result = remove_image("/tmp/nonexistent-thuki-image.jpg");
        assert!(result.is_ok());
    }

    #[test]
    fn cleanup_orphaned_images_removes_unreferenced_dirs() {
        let base = temp_dir();
        save_image(&base, "saved-conv", &tiny_png()).unwrap();
        save_image(&base, "orphan-conv", &tiny_png()).unwrap();

        let saved_ids = vec!["saved-conv".to_string()];
        let removed = cleanup_orphaned_images(&base, &saved_ids).unwrap();

        assert_eq!(removed, 1);
        assert!(conversation_dir(&base, "saved-conv").exists());
        assert!(!conversation_dir(&base, "orphan-conv").exists());

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn cleanup_orphaned_images_noop_when_no_images_dir() {
        let base = temp_dir();
        // Don't create any images directory.
        let removed = cleanup_orphaned_images(&base, &[]).unwrap();
        assert_eq!(removed, 0);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn cleanup_orphaned_images_removes_all_when_no_saved_ids() {
        let base = temp_dir();
        save_image(&base, "conv-a", &tiny_png()).unwrap();
        save_image(&base, "conv-b", &tiny_png()).unwrap();

        let removed = cleanup_orphaned_images(&base, &[]).unwrap();
        assert_eq!(removed, 2);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn cleanup_orphaned_images_preserves_all_when_all_saved() {
        let base = temp_dir();
        save_image(&base, "c1", &tiny_png()).unwrap();
        save_image(&base, "c2", &tiny_png()).unwrap();

        let saved_ids = vec!["c1".to_string(), "c2".to_string()];
        let removed = cleanup_orphaned_images(&base, &saved_ids).unwrap();
        assert_eq!(removed, 0);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn encode_images_as_base64_roundtrip() {
        let base = temp_dir();
        let path = save_image(&base, "conv-b64", &tiny_png()).unwrap();

        let encoded = encode_images_as_base64(&[path.clone()]).unwrap();
        assert_eq!(encoded.len(), 1);

        // Verify the base64 decodes to valid JPEG bytes.
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
    fn conversation_dir_resolves_correctly() {
        let base = Path::new("/tmp/thuki-test");
        assert_eq!(
            conversation_dir(base, "abc-123"),
            PathBuf::from("/tmp/thuki-test/images/abc-123")
        );
    }

    #[test]
    fn max_images_per_message_is_three() {
        assert_eq!(MAX_IMAGES_PER_MESSAGE, 3);
    }
}
