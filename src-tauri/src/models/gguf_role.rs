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
 *
 * Non-chat architectures (embedding, encoder-only) are denied at finalize/load
 * when the header reports them; missing architecture stays soft (filename path)
 * so list-time and incomplete headers never brick installs.
 */

use crate::models::gguf::GgufMetadata;

/// Role of a GGUF file relative to Thuki's chat load path (`llama-server -m`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GgufRole {
    /// Chat / completion weights eligible for `-m` when the engine supports them.
    Primary,
    /// Vision projector (`--mmproj`); never a standalone chat install.
    Projector,
    /// Non-chat companion (draft, MTP, dspark, adapter, imatrix, embedder, etc.).
    Helper,
}

/// Architectures that must never load as primary chat under the pinned engine.
///
/// Pragmatic denylist of known non-chat families (embedding, encoder-only,
/// audio tokenizers). Chat arches (llama*, gemma*, qwen*, phi*, mistral*,
/// deepseek*, gpt-oss, ...) are intentionally absent so they pass. When
/// architecture is missing, validation skips this list and keeps the soft
/// name path. Update when the engine pin adds families that should stay out
/// of the primary chat load path.
pub const DENIED_PRIMARY_ARCHES: &[&str] = &[
    "bert",
    "nomic-bert",
    "jina-bert-v2",
    "jina-bert-v3",
    "jina-bert",
    "t5",
    "t5encoder",
    "clip",
    "wavtokenizer-dec",
    "wavtokenizer",
    "roberta",
    "jina-code-embeddings",
];

/// Classifies a GGUF using optional header metadata plus the file name.
///
/// Metadata wins when it clearly identifies projector, adapter, or non-chat
/// embed arches. Otherwise the file name is inspected with the soft heuristics
/// in [`role_from_filename`]. Unknown / empty metadata with a normal quant name
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

/// True when `architecture` is a known non-chat primary (embedding / encoder).
///
/// Empty or unknown arches return false (soft path: do not block missing headers).
pub fn is_denied_primary_arch(architecture: &str) -> bool {
    let a = architecture.trim().to_ascii_lowercase();
    if a.is_empty() {
        return false;
    }
    DENIED_PRIMARY_ARCHES.iter().any(|d| *d == a)
}

/// Rejects denylisted architectures before Ready / primary load.
///
/// Missing architecture is allowed (soft path). `Some` + denylist hit returns a
/// clear user-facing error. Unknown arches not on the denylist still pass so a
/// new chat family is not blocked until the engine pin rejects it at start.
pub fn validate_primary_architecture(architecture: Option<&str>) -> Result<(), String> {
    let Some(raw) = architecture.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    if is_denied_primary_arch(raw) {
        return Err(format!(
            "\"{raw}\" is not a chat model architecture Thuki can load as a primary model. \
             Download a text-generation GGUF (llama, gemma, qwen, phi, mistral, ...) instead."
        ));
    }
    Ok(())
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
            "\"{file_name}\" is a helper file (draft/MTP/dspark/adapter/embedder), not a chat model. \
             Download a text model GGUF instead."
        ),
    }
}

/// Approximate bit width from a quant tag in `file_name` (F16→16, Q8_0→8, Q4_K_M→4).
///
/// Used only to rank mmproj companions against primary weights. Unknown tags
/// default to 16 (common projector quant) so ranking stays stable.
pub fn quant_bits(file_name: &str) -> u32 {
    let leaf = file_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_name)
        .to_ascii_lowercase();
    let stem = leaf.strip_suffix(".gguf").unwrap_or(&leaf);
    // Rightmost hyphen segment first: `model-Q4_K_M`, `mmproj-f16`, `…-Q8_0`.
    for part in stem.split('-').rev() {
        if let Some(bits) = parse_quant_token(part) {
            return bits;
        }
    }
    // Underscore-only stems: `mmproj_f16`, `model_q4_k_m`.
    for part in stem.split('_').rev() {
        if let Some(bits) = parse_quant_token(part) {
            return bits;
        }
    }
    16
}

/// Depth of shared directory prefix between two repo-relative paths (excluding
/// the leaf file). Deeper shared trees rank higher when picking an mmproj.
pub fn shared_path_prefix_depth(a: &str, b: &str) -> usize {
    let a_parts: Vec<&str> = a.split('/').filter(|p| !p.is_empty()).collect();
    let b_parts: Vec<&str> = b.split('/').filter(|p| !p.is_empty()).collect();
    let a_dirs = if a_parts.len() > 1 {
        &a_parts[..a_parts.len() - 1]
    } else {
        &[]
    };
    let b_dirs = if b_parts.len() > 1 {
        &b_parts[..b_parts.len() - 1]
    } else {
        &[]
    };
    a_dirs
        .iter()
        .zip(b_dirs.iter())
        .take_while(|(x, y)| x == y)
        .count()
}

/// Maps GGUF header fields to a role when they are decisive.
///
/// Returns `None` when metadata is silent so the caller can fall back to the
/// file name. Follows llama.cpp / Ollama: `clip` and `general.type` of
/// `mmproj`/`projector` are projectors; `adapter`/`lora` and embedding arches
/// are helpers; explicit `model` type is a positive primary signal.
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
        if is_denied_primary_arch(a) {
            // Embedding / encoder arches are never chat primaries.
            return Some(GgufRole::Helper);
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
/// Projector: `mmproj` substring, or careful CLIP leaf forms (`clip-…`,
/// `-clip-`, starts with `clip`). Helper: tokenized draft/MTP/dspark/imatrix/lora
/// plus underscore forms (`mtp_`, `draft_`). Basename is lowercased; directory
/// prefixes are stripped for matching.
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
    if leaf.contains("mmproj") {
        return GgufRole::Projector;
    }
    // CLIP projector soft names: avoid bare `contains("clip")` false positives
    // (e.g. unrelated tokens). Prefer prefix / segment forms.
    if is_clip_projector_leaf(&leaf) {
        return GgufRole::Projector;
    }
    // Speculative-decode / quant tooling companions are not chat weights.
    if leaf.contains("dspark")
        || leaf.starts_with("mtp-")
        || leaf.starts_with("mtp_")
        || leaf.contains("-mtp-")
        || leaf.contains("_mtp_")
        || leaf.contains("mtp.")
        || leaf.starts_with("draft-")
        || leaf.starts_with("draft_")
        || leaf.contains("-draft-")
        || leaf.contains("_draft_")
        || leaf.contains("imatrix")
        || has_name_token(&leaf, "lora")
        || leaf.ends_with(".gguf_file")
    {
        return GgufRole::Helper;
    }
    GgufRole::Primary
}

/// True when the lowercased leaf looks like a CLIP projector file name.
fn is_clip_projector_leaf(leaf: &str) -> bool {
    leaf.starts_with("clip-")
        || leaf.starts_with("clip_")
        || leaf.contains("-clip-")
        || leaf.contains("_clip_")
        || leaf.contains("-clip.")
        || leaf.contains("_clip.")
}

/// True when `token` appears as a whole path/name segment of `leaf` (split on
/// non-alphanumeric except underscore kept inside segments via `-`/`.` only).
///
/// Tokenizing avoids treating `lora` as a substring of an unrelated word while
/// still matching `model-lora.gguf` and `lora-adapter.gguf`.
fn has_name_token(leaf: &str, token: &str) -> bool {
    leaf.split(['-', '.', '/']).any(|part| {
        if part == token {
            return true;
        }
        // Underscore-joined: `foo_lora_bar`, `lora_weights`.
        part.split('_').any(|p| p == token)
    })
}

/// Parses one filename segment into a bit width when it is a quant tag.
fn parse_quant_token(tok: &str) -> Option<u32> {
    let t = tok.to_ascii_lowercase();
    match t.as_str() {
        "f32" | "fp32" => return Some(32),
        "f16" | "fp16" | "bf16" => return Some(16),
        "f8" | "fp8" => return Some(8),
        _ => {}
    }
    // Q#… or IQ#… (Q4_K_M, IQ2_XXS, Q8_0, …). Prefer `iq` before `q`.
    let rest = t.strip_prefix("iq").or_else(|| t.strip_prefix('q'))?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().filter(|&n: &u32| (1..=32).contains(&n))
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
            "mtp_gemma_draft.gguf",
            "draft_small_Q4.gguf",
            "model_mtp_weights.gguf",
        ] {
            assert_eq!(classify_gguf_role(name, None), GgufRole::Helper, "{name}");
            assert!(!is_chat_download_candidate(name), "{name}");
        }
    }

    #[test]
    fn clip_soft_names_are_projector_without_false_positives() {
        assert_eq!(
            classify_gguf_role("clip-model-f16.gguf", None),
            GgufRole::Projector
        );
        assert_eq!(
            classify_gguf_role("vision-clip-f16.gguf", None),
            GgufRole::Projector
        );
        // Bare substring in an unrelated token must not force projector.
        assert_eq!(
            classify_gguf_role("eclipse-Q4_K_M.gguf", None),
            GgufRole::Primary
        );
    }

    #[test]
    fn lora_token_not_substring() {
        assert_eq!(
            classify_gguf_role("model-lora.gguf", None),
            GgufRole::Helper
        );
        assert_eq!(
            classify_gguf_role("lora-adapter.gguf", None),
            GgufRole::Helper
        );
        // "floral" does not contain a lora token segment.
        assert_eq!(
            classify_gguf_role("floral-Q4_K_M.gguf", None),
            GgufRole::Primary
        );
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
    fn metadata_embed_arch_is_helper() {
        for arch in ["bert", "nomic-bert", "jina-bert-v2", "t5"] {
            let m = meta(Some(arch), None);
            assert_eq!(
                classify_gguf_role("looks-primary-Q4.gguf", Some(&m)),
                GgufRole::Helper,
                "{arch}"
            );
        }
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
    fn deny_arch_rejects_bert_allows_missing_and_chat() {
        assert!(validate_primary_architecture(None).is_ok());
        assert!(validate_primary_architecture(Some("")).is_ok());
        assert!(validate_primary_architecture(Some("qwen3")).is_ok());
        assert!(validate_primary_architecture(Some("gemma3")).is_ok());
        assert!(validate_primary_architecture(Some("llama")).is_ok());
        let err = validate_primary_architecture(Some("bert")).unwrap_err();
        assert!(err.contains("not a chat model architecture"), "{err}");
        assert!(is_denied_primary_arch("nomic-bert"));
        assert!(!is_denied_primary_arch(""));
        assert!(!is_denied_primary_arch("gpt-oss"));
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

    #[test]
    fn quant_bits_parses_common_tags() {
        assert_eq!(quant_bits("model-Q4_K_M.gguf"), 4);
        assert_eq!(quant_bits("mmproj-f16.gguf"), 16);
        assert_eq!(quant_bits("mmproj-BF16.gguf"), 16);
        assert_eq!(quant_bits("Bonsai-27B-mmproj-Q8_0.gguf"), 8);
        assert_eq!(quant_bits("weights-IQ2_XXS.gguf"), 2);
        assert_eq!(quant_bits("model-F32.gguf"), 32);
        // Underscore-only stem (no hyphens) still finds Q4.
        assert_eq!(quant_bits("model_q4_k_m.gguf"), 4);
        // Unknown → default 16.
        assert_eq!(quant_bits("mystery.gguf"), 16);
    }

    #[test]
    fn shared_path_prefix_depth_counts_dirs() {
        assert_eq!(
            shared_path_prefix_depth("mmproj/f16.gguf", "mmproj/other.gguf"),
            1
        );
        assert_eq!(
            shared_path_prefix_depth(
                "models/gemma/Q4/weights.gguf",
                "models/gemma/mmproj-f16.gguf"
            ),
            2
        );
        assert_eq!(shared_path_prefix_depth("a.gguf", "b.gguf"), 0);
        assert_eq!(shared_path_prefix_depth("x/a.gguf", "y/b.gguf"), 0);
    }
}
