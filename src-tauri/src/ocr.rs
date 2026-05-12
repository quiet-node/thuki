/*!
 * OCR (Optical Character Recognition) via macOS Vision framework.
 *
 * Exposes one Tauri command:
 *   `extract_text_command`: accepts a list of absolute image file paths,
 *   runs VNRecognizeTextRequest on each, and returns the combined extracted
 *   text. Returns "[No text detected]" when all images are blank.
 *
 * Pure helpers `process_raw_text` and `join_ocr_results` are extracted from
 * the FFI wrapper so they can be unit-tested without a running display server.
 * The FFI wrapper and command are excluded from coverage (thin I/O wrappers
 * over Vision framework; require Screen Recording permission and a display).
 */

// ─── Pure testable helpers ───────────────────────────────────────────────────

/// Converts a raw OCR string into a non-empty result, or `None` when blank.
pub fn process_raw_text(raw: String) -> Option<String> {
    if raw.trim().is_empty() {
        None
    } else {
        Some(raw)
    }
}

/// Joins per-image OCR results with a visual separator.
/// Returns "[No text detected]" when no image produced any text.
pub fn join_ocr_results(parts: Vec<String>) -> String {
    if parts.is_empty() {
        "[No text detected]".to_string()
    } else {
        parts.join("\n\n---\n\n")
    }
}

// ─── Vision FFI (macOS only) ─────────────────────────────────────────────────

// Vision framework is linked transitively via objc2-vision.
// VNRequestTextRecognitionLevelAccurate = 0 (from Vision/VNDefines.h).
const VN_RECOGNITION_LEVEL_ACCURATE: u64 = 0;

/// Calls VNRecognizeTextRequest for a single image path.
/// Returns the extracted text, or None if the image contains no recognizable text.
/// Excluded from coverage: thin wrapper over macOS Vision FFI that requires
/// a running display server and Screen Recording permission.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
fn extract_text_raw(path: &str) -> Result<Option<String>, String> {
    use objc2::rc::{autoreleasepool, Retained};
    use objc2::runtime::NSObject;
    use objc2::{class, msg_send};
    use objc2_foundation::{NSArray, NSDictionary, NSString, NSURL};

    autoreleasepool(|_| unsafe {
        // Build a file URL from the path.
        let ns_path = NSString::from_str(path);
        let url: Retained<NSURL> = NSURL::fileURLWithPath(&ns_path);

        // VNRecognizeTextRequest.new: returns +1 retained; from_raw wraps without extra retain.
        let req_raw: *mut NSObject = msg_send![class!(VNRecognizeTextRequest), new];
        let request: Retained<NSObject> =
            Retained::from_raw(req_raw).ok_or("VNRecognizeTextRequest new returned nil")?;
        let _: () = msg_send![&*request, setRecognitionLevel: VN_RECOGNITION_LEVEL_ACCURATE];

        // Wrap request in NSArray for the handler call.
        let request_ref: &NSObject = &request;
        let requests: Retained<NSArray<NSObject>> = NSArray::from_slice(&[request_ref]);

        // VNImageRequestHandler alloc + initWithURL:options:
        let options = NSDictionary::<NSString, NSObject>::new();
        let alloc: *mut NSObject = msg_send![class!(VNImageRequestHandler), alloc];
        let handler_raw: *mut NSObject = msg_send![
            alloc,
            initWithURL: &*url,
            options: &*options
        ];
        let handler: Retained<NSObject> =
            Retained::from_raw(handler_raw).ok_or("VNImageRequestHandler init returned nil")?;

        // performRequests:error:  (returns BOOL; error out-param set on failure)
        let mut error: *mut NSObject = std::ptr::null_mut();
        let ok: bool = msg_send![
            &*handler,
            performRequests: &*requests,
            error: &mut error
        ];
        if !ok {
            let msg = if error.is_null() {
                "Vision OCR failed".to_string()
            } else {
                let desc_raw: *mut NSString = msg_send![error, localizedDescription];
                if desc_raw.is_null() {
                    "Vision OCR failed".to_string()
                } else {
                    (*desc_raw).to_string()
                }
            };
            return Err(msg);
        }

        // Extract results from the request.
        let results: *mut NSObject = msg_send![&*request, results];
        if results.is_null() {
            return Ok(None);
        }
        let count: usize = msg_send![results, count];

        let mut lines: Vec<String> = Vec::with_capacity(count);
        for i in 0..count {
            let obs: *mut NSObject = msg_send![results, objectAtIndex: i];
            if obs.is_null() {
                continue;
            }
            let candidates: *mut NSObject = msg_send![obs, topCandidates: 1usize];
            if candidates.is_null() {
                continue;
            }
            let cand_count: usize = msg_send![candidates, count];
            if cand_count == 0 {
                continue;
            }
            let cand: *mut NSObject = msg_send![candidates, objectAtIndex: 0usize];
            if cand.is_null() {
                continue;
            }
            let text: *mut NSString = msg_send![cand, string];
            if !text.is_null() {
                lines.push((*text).to_string());
            }
        }

        let joined = lines.join("\n");
        Ok(process_raw_text(joined))
    })
}

/// Extracts text from each image path and joins the results.
/// Excluded from coverage: calls Vision FFI per path; logic covered by
/// the testable helpers `process_raw_text` and `join_ocr_results`.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn extract_text_from_paths(paths: &[String]) -> Result<String, String> {
    let mut parts = Vec::new();
    for path in paths {
        if let Some(text) = extract_text_raw(path)? {
            parts.push(text);
        }
    }
    Ok(join_ocr_results(parts))
}

// ─── Tauri command ───────────────────────────────────────────────────────────

/// Extracts text from one or more image files using macOS Vision OCR.
/// Excluded from coverage: thin command wrapper over `extract_text_from_paths`.
#[cfg(target_os = "macos")]
#[cfg_attr(coverage_nightly, coverage(off))]
#[tauri::command]
pub async fn extract_text_command(image_paths: Vec<String>) -> Result<String, String> {
    extract_text_from_paths(&image_paths)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // process_raw_text

    #[test]
    fn process_raw_text_returns_some_for_non_empty() {
        assert_eq!(
            process_raw_text("hello world".to_string()),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn process_raw_text_returns_none_for_empty() {
        assert_eq!(process_raw_text(String::new()), None);
    }

    #[test]
    fn process_raw_text_returns_none_for_whitespace_only() {
        assert_eq!(process_raw_text("   \t\n  ".to_string()), None);
    }

    #[test]
    fn process_raw_text_preserves_leading_trailing_whitespace_in_non_blank() {
        let input = "  hello  ".to_string();
        assert_eq!(process_raw_text(input.clone()), Some(input));
    }

    // join_ocr_results

    #[test]
    fn join_ocr_results_returns_no_text_for_empty_vec() {
        assert_eq!(join_ocr_results(vec![]), "[No text detected]".to_string());
    }

    #[test]
    fn join_ocr_results_returns_single_item_unchanged() {
        assert_eq!(
            join_ocr_results(vec!["hello".to_string()]),
            "hello".to_string()
        );
    }

    #[test]
    fn join_ocr_results_joins_multiple_with_separator() {
        let result = join_ocr_results(vec!["first".to_string(), "second".to_string()]);
        assert_eq!(result, "first\n\n---\n\nsecond".to_string());
    }

    #[test]
    fn join_ocr_results_joins_three_parts() {
        let result = join_ocr_results(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        assert_eq!(result, "a\n\n---\n\nb\n\n---\n\nc".to_string());
    }
}
