/*!
 * GGUF artifact role classification for browse/install/load gates.
 *
 * Hugging Face multi-file repos ship chat weights next to vision projectors
 * (mmproj / CLIP) and optional helpers (draft / MTP / dspark). Filename denylists
 * rot: `mmproj*.gguf` misses `Bonsai-27B-mmproj-Q8_0.gguf`. This module is the
 * single policy for "what may be a primary chat model" vs companion vs helper.
 *
 * Order of signals: GGUF metadata (architecture / general.type) first when
 * present; filename only as a soft fallback for list-time (no local blob yet).
 * Draft/MTP/dspark are never auto-wired to the engine; they are only excluded
 * from primary chat download/Ready.
 */

use crate::models::gguf::GgufMetadata;

/// Role of a GGUF file relative to Thuki's chat load path (`llama-server -m`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GgufRole {
    /// Chat / completion weights eligible for `-m` when the engine supports them.
    Primary,
    /// Vision projector (`--mmproj`); never a standalone chat install.
    Projector,
    /// Non-chat companion (draft, MTP, dspark, adapter, imatrix, etc.).
    Helper,
}

/// Classifies a GGUF using optional header metadata plus the file name.
///
/// Metadata wins when it clearly identifies projector or adapter roles.
/// Otherwise the file name is inspected with the soft heuristics in
/// [`role_from_filename`]. Unknown / empty metadata with a normal quant name
/// defaults to [`GgufRole::Primary`] so list-time and legacy brains still work.
pub fn classify_gguf_role(file_name: &str, meta: Option<&GgufMetadata>) -> GgufRole {
    if let Some(m) = meta {
        if let Some(role) = role_from_metadata(m) {
            return role;
        }
    }
    role_from_filename(file_name)
}

/// True when `file_name` may appear as a Browse-all / paste-repo chat download.
///
/// List-time has no local header: uses filename soft classification only.
pub fn is_chat_download_candidate(file_name: &str) -> bool {
    matches!(role_from_filename(file_name), GgufRole::Primary)
}

/// True when `file_name` is a vision projector companion candidate at list/resolve
/// time (filename soft signals). Used to auto-attach alongside a brain install.
pub fn is_projector_companion_name(file_name: &str) -> bool {
    matches!(role_from_filename(file_name), GgufRole::Projector)
}

/// Rejects non-primary roles before a blob becomes Ready / primary load.
///
/// Returns `Ok(())` for [`GgufRole::Primary`]. Errors carry user-facing copy that
/// names projector vs helper so the UI is not a vague engine-start failure.
pub fn validate_primary_weights_role(
    file_name: &str,
    meta: Option<&GgufMetadata>,
) -> Result<(), String> {
    match classify_gguf_role(file_name, meta) {
        GgufRole::Primary => Ok(()),
        GgufRole::Projector => Err(primary_role_error(GgufRole::Projector, file_name)),
        GgufRole::Helper => Err(primary_role_error(GgufRole::Helper, file_name)),
    }
}

/// User-facing message when a non-primary artifact is treated as a chat model.
pub fn primary_role_error(role: GgufRole, file_name: &str) -> String {
    match role {
        GgufRole::Primary => String::new(),
        GgufRole::Projector => format!(
            "\"{file_name}\" is a vision projector (CLIP/mmproj), not a chat model. \
             Download a text model GGUF from the same repo; Thuki attaches the projector automatically."
        ),
        GgufRole::Helper => format!(
            "\"{file_name}\" is a helper file (draft/MTP/dspark/adapter), not a chat model. \
             Download a text model GGUF instead."
        ),
    }
}

/// Maps GGUF header fields to a role when they are decisive.
///
/// Returns `None` when metadata is silent so the caller can fall back to the
/// file name. Follows the llama.cpp / Ollama convention: `clip` and
/// `general.type` of `mmproj`/`projector` are projectors; `adapter` is a helper.
fn role_from_metadata(meta: &GgufMetadata) -> Option<GgufRole> {
    let arch = meta
        .architecture
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());
    let gtype = meta
        .general_type
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase());

    if let Some(ref t) = gtype {
        if t == "mmproj" || t == "projector" {
            return Some(GgufRole::Projector);
        }
        if t == "adapter" || t == "lora" {
            return Some(GgufRole::Helper);
        }
    }
    if let Some(ref a) = arch {
        if a == "clip" {
            return Some(GgufRole::Projector);
        }
    }
    // Explicit model type is a positive primary signal when present.
    if gtype.as_deref() == Some("model") {
        return Some(GgufRole::Primary);
    }
    None
}

/// Soft role from the leaf file name only (list-time / missing header).
///
/// Projector: `mmproj` as a path segment or substring (`mmproj-f16.gguf`,
/// `Bonsai-27B-mmproj-Q8_0.gguf`). Helper: draft/MTP/dspark/imatrix/lora markers.
/// Basename is lowercased; directory prefixes are stripped for matching.
fn role_from_filename(file_name: &str) -> GgufRole {
    let leaf = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_name)
        .to_ascii_lowercase();
    if !leaf.ends_with(".gguf") {
        // Non-gguf never reaches browse rows; treat as non-primary if misused.
        return GgufRole::Helper;
    }
    if leaf.starts_with("mmproj") || leaf.contains("mmproj") {
        return GgufRole::Projector;
    }
    // Speculative-decode / quant tooling companions are not chat weights.
    if leaf.contains("dspark")
        || leaf.starts_with("mtp-")
        || leaf.contains("-mtp-")
        || leaf.contains("mtp.")
        || leaf.starts_with("draft-")
        || leaf.contains("-draft-")
        || leaf.contains("imatrix")
        || leaf.contains("lora")
        || leaf.ends_with(".gguf_file")
    {
        return GgufRole::Helper;
    }
    GgufRole::Primary
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(arch: Option<&str>, gtype: Option<&str>) -> GgufMetadata {
        GgufMetadata {
            chat_template: None,
            architecture: arch.map(str::to_string),
            general_type: gtype.map(str::to_string),
        }
    }

    #[test]
    fn brain_quant_is_primary_by_name() {
        assert_eq!(
            classify_gguf_role("Bonsai-27B-Q1_0.gguf", None),
            GgufRole::Primary
        );
        assert_eq!(
            classify_gguf_role("gemma-4-26B-A4B-it-UD-Q3_K_XL.gguf", None),
            GgufRole::Primary
        );
        assert!(is_chat_download_candidate("model-Q4_K_M.gguf"));
    }

    #[test]
    fn prefix_mmproj_is_projector() {
        assert_eq!(
            classify_gguf_role("mmproj-BF16.gguf", None),
            GgufRole::Projector
        );
        assert!(is_projector_companion_name("mmproj-model-f16.gguf"));
        assert!(!is_chat_download_candidate("mmproj-BF16.gguf"));
    }

    #[test]
    fn mid_name_mmproj_is_projector() {
        assert_eq!(
            classify_gguf_role("Bonsai-27B-mmproj-Q8_0.gguf", None),
            GgufRole::Projector
        );
        assert_eq!(
            classify_gguf_role("gemma-4-E4B-it-mmproj.gguf", None),
            GgufRole::Projector
        );
        assert!(is_projector_companion_name("Bonsai-27B-mmproj-BF16.gguf"));
        assert!(!is_chat_download_candidate("Bonsai-27B-mmproj-Q8_0.gguf"));
    }

    #[test]
    fn helper_names_are_not_chat_downloads() {
        for name in [
            "Bonsai-27B-dspark-Q4_1.gguf",
            "mtp-gemma-4-26B-A4B-it.gguf",
            "MTP/mtp-gemma-4-26B-A4B-it-Q8_0.gguf",
            "draft-small.gguf",
            "model-lora.gguf",
            "imatrix_unsloth.gguf",
        ] {
            assert_eq!(classify_gguf_role(name, None), GgufRole::Helper, "{name}");
            assert!(!is_chat_download_candidate(name), "{name}");
        }
    }

    #[test]
    fn metadata_clip_overrides_brain_like_name() {
        let m = meta(Some("clip"), None);
        assert_eq!(
            classify_gguf_role("looks-like-Q4_K_M.gguf", Some(&m)),
            GgufRole::Projector
        );
    }

    #[test]
    fn metadata_mmproj_type_is_projector() {
        let m = meta(Some("clip"), Some("mmproj"));
        assert_eq!(
            classify_gguf_role("weights.gguf", Some(&m)),
            GgufRole::Projector
        );
        let m2 = meta(None, Some("projector"));
        assert_eq!(
            classify_gguf_role("weights.gguf", Some(&m2)),
            GgufRole::Projector
        );
    }

    #[test]
    fn metadata_adapter_is_helper() {
        let m = meta(Some("llama"), Some("adapter"));
        assert_eq!(
            classify_gguf_role("adapter.gguf", Some(&m)),
            GgufRole::Helper
        );
    }

    #[test]
    fn metadata_model_type_is_primary() {
        let m = meta(Some("qwen3"), Some("model"));
        assert_eq!(
            classify_gguf_role("anything.gguf", Some(&m)),
            GgufRole::Primary
        );
    }

    #[test]
    fn validate_primary_rejects_projector_and_helper() {
        assert!(validate_primary_weights_role("brain-Q4_K_M.gguf", None).is_ok());
        let err = validate_primary_weights_role("Bonsai-27B-mmproj-Q8_0.gguf", None).unwrap_err();
        assert!(err.contains("vision projector"), "{err}");
        let err = validate_primary_weights_role("Bonsai-27B-dspark-Q4_1.gguf", None).unwrap_err();
        assert!(err.contains("helper file"), "{err}");
        let clip = meta(Some("clip"), None);
        let err = validate_primary_weights_role("renamed.gguf", Some(&clip)).unwrap_err();
        assert!(err.contains("vision projector"), "{err}");
    }

    #[test]
    fn primary_role_error_covers_all_arms() {
        assert!(primary_role_error(GgufRole::Primary, "x.gguf").is_empty());
        assert!(primary_role_error(GgufRole::Projector, "p.gguf").contains("vision projector"));
        assert!(primary_role_error(GgufRole::Helper, "h.gguf").contains("helper file"));
    }

    #[test]
    fn metadata_lora_type_is_helper_and_silent_arch_falls_back_to_name() {
        let lora = meta(Some("llama"), Some("lora"));
        assert_eq!(
            classify_gguf_role("weights.gguf", Some(&lora)),
            GgufRole::Helper
        );
        // Architecture alone without a decisive type falls through to filename.
        let quiet = meta(Some("qwen3"), None);
        assert_eq!(
            classify_gguf_role("Bonsai-27B-Q1_0.gguf", Some(&quiet)),
            GgufRole::Primary
        );
        assert_eq!(
            classify_gguf_role("Bonsai-27B-mmproj-Q8_0.gguf", Some(&quiet)),
            GgufRole::Projector
        );
    }

    #[test]
    fn whitespace_only_metadata_is_ignored() {
        let m = GgufMetadata {
            chat_template: None,
            architecture: Some("   ".into()),
            general_type: Some("\t".into()),
        };
        assert_eq!(
            classify_gguf_role("brain-Q4_K_M.gguf", Some(&m)),
            GgufRole::Primary
        );
    }
}
