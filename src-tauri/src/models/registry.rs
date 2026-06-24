/*!
 * Curated model registry for the built-in llama.cpp engine.
 *
 * This is the Staff Picks catalog: a small, deeply-vetted set of models grouped
 * into use-case sections (Everyday chat / Compact & fast / Deep reasoning).
 * Three of the entries double as the onboarding heroes (one per tier, see
 * [`ONBOARDING_HERO_IDS`]); the rest exist only in the catalog. Every entry
 * pins a Hugging Face repo at an exact git revision and carries the SHA-256 of
 * each blob, so a download is reproducible and verifiable end to end (the
 * digests feed straight into [`crate::models::download::DownloadSpec`] which
 * verifies them on install). Provenance comes from the pinned revision and a
 * trusted GGUF source (the maker's own repo, `unsloth`, `bartowski`, or
 * `ggml-org`); the SHA-256 is an integrity check only.
 *
 * Hashes and sizes were read from the Hugging Face tree-at-revision API
 * (`/api/models/<repo>/tree/<revision>`): the three heroes on 2026-06-17, the
 * rest of the catalog on 2026-06-20, so each digest matches its pinned commit,
 * not whatever `main` later points to.
 */

use crate::config::defaults::HF_BASE_URL;
use crate::models::download::DownloadSpec;
use crate::models::manifest::InstalledModel;

/// Starter tier: a coarse speed/quality dial for the model picker.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Fast,
    Balanced,
    Smartest,
}

/// One curated starter model: everything the download UI and the installer
/// need, baked in at compile time.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct Starter {
    /// Stable slug, unique across the registry (e.g. `"gemma-4-12b"`). The
    /// download key and the React row key for the Staff Picks catalog, where a
    /// single category can hold many models. Onboarding keys on `tier` instead
    /// and shows only the three [`ONBOARDING_HERO_IDS`] heroes.
    pub id: &'static str,
    /// Coarse speed/quality dial for the model. Onboarding's 3-up comparison
    /// shows one hero per tier; in the Staff Picks catalog several entries can
    /// share a tier, so it is a size/speed hint there, not a unique key.
    pub tier: Tier,
    /// Model family this entry belongs to (e.g. "Gemma", "Qwen", "gpt-oss").
    /// Several starters can share a family when the catalog offers more than one
    /// size of the same model.
    pub family: &'static str,
    /// Use-case section the Discover staff-picks list groups this entry under
    /// (e.g. "Everyday chat", "Compact & fast", "Deep reasoning"). Answers
    /// "what is it for?" in plain words so a non-expert can pick by intent.
    pub category: &'static str,
    /// Human-readable label shown in the picker (e.g. "Gemma 4 12B").
    pub display_name: &'static str,
    /// Hugging Face repo slug.
    pub repo: &'static str,
    /// 40-hex git commit SHA the download is pinned to.
    pub revision: &'static str,
    /// Weights file name within the repo at that revision.
    pub file_name: &'static str,
    /// Lowercase hex SHA-256 of the weights blob.
    pub sha256: &'static str,
    /// Weights file size in bytes.
    pub size_bytes: u64,
    /// Quantisation label (e.g. "Q4_K_M").
    pub quant: &'static str,
    /// Whether the model accepts image inputs.
    pub vision: bool,
    /// Whether the model emits a thinking/scratchpad token stream.
    pub thinking: bool,
    /// Whether the model's reasoning cannot be turned off (it always reasons).
    /// `true` only for structurally-always-on families (e.g. gpt-oss/Harmony);
    /// `false` when reasoning is optional (the default-off path) or absent.
    pub reasoning_always: bool,
    /// Vision projection file name, when the model is multimodal.
    pub mmproj_file: Option<&'static str>,
    /// Lowercase hex SHA-256 of the mmproj blob, when present.
    pub mmproj_sha256: Option<&'static str>,
    /// mmproj file size in bytes; 0 exactly when `mmproj_file` is `None`.
    pub mmproj_bytes: u64,
    /// Estimated resident memory in GiB, roughly
    /// `(size_bytes + mmproj_bytes) / 2^30` plus the 16k-context KV cache
    /// (sized from the model's layer/head geometry under llama.cpp's default
    /// sliding-window-aware cache). Sanity-check any new entry against a
    /// real load before trusting the estimate.
    pub est_runtime_gb: f64,
    /// Maximum context window in tokens the model was trained for: its GGUF
    /// `context_length` metadata (llama.cpp's `n_ctx_train`), vetted against the
    /// maker's published config. Surfaced in the picker so a user can see how
    /// much a model can attend to. Display only: the engine loads the user's
    /// separate, clamped `num_ctx`, never this value.
    pub context_length: u32,
    /// Short license label surfaced next to the download button.
    pub license_note: &'static str,
    /// Model maker (e.g. "OpenAI"), shown in the picker's Origin row.
    pub origin: &'static str,
    /// The maker's own official Hugging Face repo, opened from the Origin row
    /// so a user can verify provenance on the source org's page. Differs from
    /// `repo` (the GGUF download source) when a third party hosts the GGUF.
    pub origin_repo: &'static str,
}

/// The curated starters, ordered Fast, Balanced, Smartest.
pub const STARTERS: &[Starter] = &[
    Starter {
        id: "qwen3.5-9b",
        tier: Tier::Fast,
        family: "Qwen",
        category: "Everyday chat",
        display_name: "Qwen3.5 9B",
        repo: "unsloth/Qwen3.5-9B-GGUF",
        revision: "3885219b6810b007914f3a7950a8d1b469d598a5",
        file_name: "Qwen3.5-9B-Q4_K_M.gguf",
        sha256: "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8",
        size_bytes: 5_680_522_464,
        quant: "Q4_K_M",
        vision: true,
        thinking: true,
        reasoning_always: false,
        mmproj_file: Some("mmproj-BF16.gguf"),
        mmproj_sha256: Some("853698ce7aa6c7ba732478bad280240969ddf7b0fcbf93900046f63903a83383"),
        mmproj_bytes: 921_705_024,
        est_runtime_gb: 8.5,
        context_length: 262_144,
        license_note: "Apache 2.0",
        origin: "Alibaba",
        origin_repo: "Qwen/Qwen3.5-9B",
    },
    Starter {
        id: "gemma-4-12b",
        tier: Tier::Balanced,
        family: "Gemma",
        category: "Everyday chat",
        display_name: "Gemma 4 12B",
        repo: "google/gemma-4-12B-it-qat-q4_0-gguf",
        revision: "f6e7774e6148da3b7f201e42ba37cf084c1db35f",
        file_name: "gemma-4-12b-it-qat-q4_0.gguf",
        sha256: "faff1a63667fac17ac5e777f47114688fcefea96e220e211aaa8d62c2c4561f1",
        size_bytes: 6_975_877_728,
        quant: "Q4_0",
        vision: true,
        // Gemma 4 supports an optional thinking mode toggled via the chat
        // template's `enable_thinking` kwarg (default off), so reasoning is
        // on-demand, like Qwen3.5.
        thinking: true,
        reasoning_always: false,
        mmproj_file: Some("mmproj-gemma-4-12b-it-qat-q4_0.gguf"),
        mmproj_sha256: Some("e70b0e5cd80323d5d588b4ed06780356b7b1ba03995a4b8164c6ae9db0ff5989"),
        mmproj_bytes: 175_115_264,
        est_runtime_gb: 9.5,
        context_length: 262_144,
        license_note: "Apache 2.0",
        origin: "Google",
        origin_repo: "google/gemma-4-12B-it",
    },
    Starter {
        id: "gpt-oss-20b",
        tier: Tier::Smartest,
        family: "gpt-oss",
        category: "Deep reasoning",
        display_name: "gpt-oss 20B",
        repo: "ggml-org/gpt-oss-20b-GGUF",
        revision: "e1dc459feff949ff451ce107337a2026daa80df8",
        file_name: "gpt-oss-20b-mxfp4.gguf",
        sha256: "be37a636aca0fc1aae0d32325f82f6b4d21495f06823b5fbc1898ae0303e9935",
        size_bytes: 12_109_566_560,
        quant: "MXFP4",
        vision: false,
        thinking: true,
        reasoning_always: true,
        mmproj_file: None,
        mmproj_sha256: None,
        mmproj_bytes: 0,
        est_runtime_gb: 13.3,
        context_length: 131_072,
        license_note: "Apache 2.0",
        origin: "OpenAI",
        origin_repo: "openai/gpt-oss-20b",
    },
    // ── Everyday chat ──────────────────────────────────────────────────────
    Starter {
        id: "mistral-nemo-12b",
        tier: Tier::Balanced,
        family: "Mistral",
        category: "Everyday chat",
        display_name: "Mistral Nemo 12B",
        repo: "bartowski/Mistral-Nemo-Instruct-2407-GGUF",
        revision: "a2dd64a0a76ea1bdb2bb6ab6fa5496b003c7c908",
        file_name: "Mistral-Nemo-Instruct-2407-Q4_K_M.gguf",
        sha256: "7c1a10d202d8788dbe5628dc962254d10654c853cae6aaeca0618f05490d4a46",
        size_bytes: 7_477_208_192,
        quant: "Q4_K_M",
        vision: false,
        thinking: false,
        reasoning_always: false,
        mmproj_file: None,
        mmproj_sha256: None,
        mmproj_bytes: 0,
        est_runtime_gb: 9.9,
        context_length: 131_072,
        license_note: "Apache 2.0",
        origin: "Mistral",
        origin_repo: "mistralai/Mistral-Nemo-Instruct-2407",
    },
    // ── Compact & fast ─────────────────────────────────────────────────────
    Starter {
        id: "phi-4-mini-3.8b",
        tier: Tier::Fast,
        family: "Phi",
        category: "Compact & fast",
        display_name: "Phi-4 Mini 3.8B",
        repo: "unsloth/Phi-4-mini-instruct-GGUF",
        revision: "78eb92a46fc37e6b524df991ed9aca9bc6aa7b80",
        file_name: "Phi-4-mini-instruct-Q4_K_M.gguf",
        sha256: "88c00229914083cd112853aab84ed51b87bdf6b9ce42f532d8c85c7c63b1730a",
        size_bytes: 2_491_874_272,
        quant: "Q4_K_M",
        vision: false,
        thinking: false,
        reasoning_always: false,
        mmproj_file: None,
        mmproj_sha256: None,
        mmproj_bytes: 0,
        est_runtime_gb: 4.7,
        context_length: 131_072,
        license_note: "MIT",
        origin: "Microsoft",
        origin_repo: "microsoft/Phi-4-mini-instruct",
    },
    Starter {
        id: "llama-3.2-3b",
        tier: Tier::Fast,
        family: "Llama",
        category: "Compact & fast",
        display_name: "Llama 3.2 3B",
        repo: "bartowski/Llama-3.2-3B-Instruct-GGUF",
        revision: "5ab33fa94d1d04e903623ae72c95d1696f09f9e8",
        file_name: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        sha256: "6c1a2b41161032677be168d354123594c0e6e67d2b9227c84f296ad037c728ff",
        size_bytes: 2_019_377_696,
        quant: "Q4_K_M",
        vision: false,
        thinking: false,
        reasoning_always: false,
        mmproj_file: None,
        mmproj_sha256: None,
        mmproj_bytes: 0,
        est_runtime_gb: 4.0,
        context_length: 131_072,
        license_note: "Llama 3.2 Community",
        origin: "Meta",
        origin_repo: "meta-llama/Llama-3.2-3B-Instruct",
    },
    Starter {
        id: "gemma-4-e4b",
        tier: Tier::Fast,
        family: "Gemma",
        category: "Compact & fast",
        display_name: "Gemma 4 E4B",
        repo: "google/gemma-4-E4B-it-qat-q4_0-gguf",
        revision: "bb3b92e6f031fa438b409f898dd9f14f499a0cb0",
        file_name: "gemma-4-E4B_q4_0-it.gguf",
        sha256: "e8b6a059ba86947a44ace84d6e5679795bc41862c25c30513142588f0e9dba1d",
        size_bytes: 5_154_939_136,
        quant: "Q4_0",
        vision: true,
        thinking: false,
        reasoning_always: false,
        mmproj_file: Some("gemma-4-E4B-it-mmproj.gguf"),
        mmproj_sha256: Some("c6398448d84a4836fdedf58f9775979e69ae0cc4dfdf4d697b5597693a555b12"),
        mmproj_bytes: 991_551_904,
        est_runtime_gb: 7.4,
        context_length: 131_072,
        license_note: "Gemma",
        origin: "Google",
        origin_repo: "google/gemma-4-E4B-it",
    },
    // ── Deep reasoning ─────────────────────────────────────────────────────
    Starter {
        id: "phi-4-reasoning-plus-14b",
        tier: Tier::Smartest,
        family: "Phi",
        category: "Deep reasoning",
        display_name: "Phi-4 Reasoning Plus 14B",
        repo: "unsloth/Phi-4-reasoning-plus-GGUF",
        revision: "80fff8542dc7b88dba725b660beefd80e91e80c9",
        file_name: "Phi-4-reasoning-plus-Q4_K_M.gguf",
        sha256: "faf720745e20df40f52ee218be14c72b33070f7aacc508b3fbc61d47f32b4ffe",
        size_bytes: 9_053_117_120,
        quant: "Q4_K_M",
        vision: false,
        thinking: true,
        reasoning_always: true,
        mmproj_file: None,
        mmproj_sha256: None,
        mmproj_bytes: 0,
        est_runtime_gb: 12.0,
        context_length: 32_768,
        license_note: "MIT",
        origin: "Microsoft",
        origin_repo: "microsoft/Phi-4-reasoning-plus",
    },
    Starter {
        id: "deepseek-r1-distill-8b",
        tier: Tier::Balanced,
        family: "DeepSeek",
        category: "Deep reasoning",
        display_name: "DeepSeek-R1 Distill 8B",
        repo: "unsloth/DeepSeek-R1-Distill-Llama-8B-GGUF",
        revision: "615f8936e16dfde29dcc00be71145d4d5ce8ed53",
        file_name: "DeepSeek-R1-Distill-Llama-8B-Q4_K_M.gguf",
        sha256: "0addb1339a82385bcd973186cd80d18dcc71885d45eabd899781a118d03827d9",
        size_bytes: 4_920_737_216,
        quant: "Q4_K_M",
        vision: false,
        thinking: true,
        reasoning_always: true,
        mmproj_file: None,
        mmproj_sha256: None,
        mmproj_bytes: 0,
        est_runtime_gb: 7.0,
        context_length: 131_072,
        license_note: "MIT",
        origin: "DeepSeek",
        origin_repo: "deepseek-ai/DeepSeek-R1-Distill-Llama-8B",
    },
];

/// Ids of the three onboarding hero starters, in tier order
/// (Fast, Balanced, Smartest). Onboarding's 3-up comparison selects exactly
/// these by id; the Staff Picks catalog may hold any number of other entries
/// without disturbing the onboarding heroes.
pub const ONBOARDING_HERO_IDS: [&str; 3] = ["qwen3.5-9b", "gemma-4-12b", "gpt-oss-20b"];

/// The registry entry with this id, if any. The id-keyed download path and the
/// onboarding-hero lookup both resolve entries through here, so a bad id yields
/// `None` rather than a panic.
pub fn by_id(id: &str) -> Option<&'static Starter> {
    STARTERS.iter().find(|s| s.id == id)
}

/// The registry entry matching this repo + weights file name, if any. An
/// installed model heals its curated facts (capabilities, context window) from
/// the registry through here, so a later flag or pin correction reaches models
/// downloaded before it. A pasted (non-curated) repo has no entry and yields
/// `None`.
pub fn by_repo_file(repo: &str, file_name: &str) -> Option<&'static Starter> {
    STARTERS
        .iter()
        .find(|s| s.repo == repo && s.file_name == file_name)
}

/// The three onboarding hero starters, resolved from [`ONBOARDING_HERO_IDS`] in
/// tier order. Any id that is absent from the registry is skipped, so the
/// result is the heroes that actually exist; a registry test asserts all three
/// resolve, so in practice the list is always length three.
pub fn onboarding_heroes() -> Vec<&'static Starter> {
    ONBOARDING_HERO_IDS
        .iter()
        .filter_map(|id| by_id(id))
        .collect()
}

/// RAM-fit hint rendered as a badge on each starter row.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RamFit {
    Fits,
    Tight,
    TooBig,
}

/// RAM-fit hint. `ram_bytes` is hw.memsize; GiB = bytes / 2^30.
/// fits when est <= 0.60 * ram_gib; tight when <= 0.85 * ram_gib; too_big above.
/// 60% leaves headroom for the OS and other apps in unified memory; up to 85%
/// runs but close to the machine's limit; beyond that macOS swaps heavily.
pub fn ram_fit(est_runtime_gb: f64, ram_bytes: u64) -> RamFit {
    let ram_gib = ram_bytes as f64 / (1u64 << 30) as f64;
    if est_runtime_gb <= 0.60 * ram_gib {
        RamFit::Fits
    } else if est_runtime_gb <= 0.85 * ram_gib {
        RamFit::Tight
    } else {
        RamFit::TooBig
    }
}

/// Download URL: `{HF_BASE_URL}/<repo>/resolve/<revision>/<file>`.
/// One spec for the weights, plus one for the mmproj when present.
pub fn download_specs(s: &Starter) -> Vec<DownloadSpec> {
    let url = |file: &str| format!("{}/{}/resolve/{}/{}", HF_BASE_URL, s.repo, s.revision, file);
    let mut specs = vec![DownloadSpec {
        url: url(s.file_name),
        file: s.file_name.to_string(),
        sha256: s.sha256.to_string(),
        total_bytes: s.size_bytes,
    }];
    if let (Some(mmproj_file), Some(mmproj_sha256)) = (s.mmproj_file, s.mmproj_sha256) {
        specs.push(DownloadSpec {
            url: url(mmproj_file),
            file: mmproj_file.to_string(),
            sha256: mmproj_sha256.to_string(),
            total_bytes: s.mmproj_bytes,
        });
    }
    specs
}

/// The manifest-row id for a starter: `"<repo>:<file_name>"`. The single source
/// of truth for how a curated entry maps onto its installed-manifest key, so the
/// installed-state probe can resolve the id without building a whole
/// [`InstalledModel`] just to read one field.
pub fn installed_model_id(s: &Starter) -> String {
    format!("{}:{}", s.repo, s.file_name)
}

/// Manifest row for an installed starter. id = [`installed_model_id`].
pub fn to_installed_model(s: &Starter) -> InstalledModel {
    InstalledModel {
        id: installed_model_id(s),
        display_name: s.display_name.to_string(),
        repo: s.repo.to_string(),
        revision: s.revision.to_string(),
        file_name: s.file_name.to_string(),
        sha256: s.sha256.to_string(),
        size_bytes: s.size_bytes,
        quant: s.quant.to_string(),
        vision: s.vision,
        thinking: s.thinking,
        reasoning_always: s.reasoning_always,
        mmproj_file: s.mmproj_file.map(str::to_string),
        mmproj_sha256: s.mmproj_sha256.map(str::to_string),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// True when `s` is exactly `len` lowercase ASCII hex chars.
    fn is_lower_hex(s: &str, len: usize) -> bool {
        s.len() == len && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    }

    /// Resolves the onboarding hero for a tier by id, not by find-first-of-tier:
    /// the catalog can hold several entries of the same tier, so only the hero
    /// ids identify the three onboarding models unambiguously.
    fn starter(tier: Tier) -> &'static Starter {
        let idx = match tier {
            Tier::Fast => 0,
            Tier::Balanced => 1,
            Tier::Smartest => 2,
        };
        by_id(ONBOARDING_HERO_IDS[idx]).unwrap()
    }

    #[test]
    fn blob_shas_are_unique_across_entries() {
        // Parallel downloads rely on no two catalog entries sharing a blob: the
        // content-addressed store would otherwise see two concurrent writers to
        // the same `tmp/<sha>.partial`. If a future entry legitimately shares a
        // blob (e.g. a common mmproj companion), add per-sha download
        // serialization before relaxing this guard. See `DownloadState` docs.
        let mut seen = std::collections::HashSet::new();
        for s in STARTERS {
            assert!(
                seen.insert(s.sha256),
                "duplicate weights sha256: {}",
                s.sha256
            );
            if let Some(mmproj) = s.mmproj_sha256 {
                assert!(seen.insert(mmproj), "duplicate blob sha256: {mmproj}");
            }
        }
    }

    #[test]
    fn ids_are_present_and_unique() {
        // The Staff Picks catalog and the id-keyed download path key on `id`,
        // so every entry needs a non-empty slug and no two may collide.
        let mut seen = std::collections::HashSet::new();
        for s in STARTERS {
            assert!(!s.id.is_empty(), "{}: id is empty", s.repo);
            assert!(seen.insert(s.id), "duplicate id: {}", s.id);
        }
    }

    #[test]
    fn by_repo_file_matches_repo_and_weights_file() {
        // Heals an installed model's curated facts from the registry: it matches
        // on repo + weights file, and misses when either differs.
        let s = &STARTERS[0];
        assert_eq!(by_repo_file(s.repo, s.file_name).unwrap().id, s.id);
        assert!(by_repo_file(s.repo, "other.gguf").is_none());
        assert!(by_repo_file("other/repo", s.file_name).is_none());
    }

    #[test]
    fn by_id_resolves_present_and_misses_unknown() {
        // by_id finds a present entry and returns None for an unknown slug,
        // so the lookup never panics on a bad id.
        assert_eq!(by_id(STARTERS[0].id).unwrap().id, STARTERS[0].id);
        assert!(by_id("no-such-model").is_none());
    }

    #[test]
    fn onboarding_heroes_are_three_in_tier_order() {
        // The onboarding picker shows exactly three heroes, one per tier, in
        // Fast/Balanced/Smartest order; each id resolves to a real entry.
        assert_eq!(ONBOARDING_HERO_IDS.len(), 3);
        let heroes = onboarding_heroes();
        assert_eq!(heroes.len(), 3);
        assert_eq!(
            heroes.iter().map(|s| s.tier).collect::<Vec<_>>(),
            vec![Tier::Fast, Tier::Balanced, Tier::Smartest]
        );
        for id in ONBOARDING_HERO_IDS {
            assert!(by_id(id).is_some(), "hero id missing from registry: {id}");
        }
    }

    #[test]
    fn catalog_is_the_vetted_models_grouped_by_category() {
        // The curated Staff Picks catalog: nine deeply-vetted models, exactly
        // three per use-case section the Discover surface renders. The three
        // onboarding heroes are among them, so a model downloaded during
        // onboarding shows up here as Installed with no duplicate row. Locks the
        // exact set so a stray add/remove is a deliberate, reviewed change.
        use std::collections::BTreeMap;
        let mut by_cat: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for s in STARTERS {
            by_cat.entry(s.category).or_default().push(s.id);
        }
        for v in by_cat.values_mut() {
            v.sort_unstable();
        }
        let mut expected: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        expected.insert(
            "Everyday chat",
            vec!["gemma-4-12b", "mistral-nemo-12b", "qwen3.5-9b"],
        );
        expected.insert(
            "Compact & fast",
            vec!["gemma-4-e4b", "llama-3.2-3b", "phi-4-mini-3.8b"],
        );
        expected.insert(
            "Deep reasoning",
            vec![
                "deepseek-r1-distill-8b",
                "gpt-oss-20b",
                "phi-4-reasoning-plus-14b",
            ],
        );
        for v in expected.values_mut() {
            v.sort_unstable();
        }
        assert_eq!(by_cat, expected);
    }

    #[test]
    fn context_windows_match_the_vetted_values() {
        // The model's trained max context (GGUF `context_length`), vetted per
        // entry against the maker's config; Mistral Nemo is corrected from its
        // GGUF's inflated 1,024,000 down to its real 131,072.
        let want: &[(&str, u32)] = &[
            ("qwen3.5-9b", 262_144),
            ("gemma-4-12b", 262_144),
            ("mistral-nemo-12b", 131_072),
            ("phi-4-mini-3.8b", 131_072),
            ("llama-3.2-3b", 131_072),
            ("gemma-4-e4b", 131_072),
            ("gpt-oss-20b", 131_072),
            ("phi-4-reasoning-plus-14b", 32_768),
            ("deepseek-r1-distill-8b", 131_072),
        ];
        for (id, ctx) in want {
            assert_eq!(
                by_id(id).unwrap().context_length,
                *ctx,
                "{id} context window"
            );
        }
    }

    #[test]
    fn every_entry_has_a_sane_context_window() {
        // Display-only trained max; a floor/ceiling guards against a typo and
        // documents that the value is bounded. The real KV allocation is the
        // user's separate, clamped `num_ctx`, never this number.
        for s in STARTERS {
            assert!(
                (2048..=1_048_576).contains(&s.context_length),
                "{}: context_length {} out of sane range",
                s.id,
                s.context_length
            );
        }
    }

    #[test]
    fn every_category_holds_exactly_three_models() {
        // The Discover surface is balanced: nine models, exactly three per
        // use-case section, so no section dwarfs the others.
        use std::collections::BTreeMap;
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for s in STARTERS {
            *counts.entry(s.category).or_default() += 1;
        }
        assert_eq!(STARTERS.len(), 9, "catalog should hold nine models");
        for (category, n) in counts {
            assert_eq!(
                n, 3,
                "category {category} should hold exactly three, has {n}"
            );
        }
    }

    #[test]
    fn every_entry_carries_origin_and_license() {
        for s in STARTERS {
            assert!(!s.license_note.is_empty(), "{}: empty license", s.id);
            assert!(!s.origin.is_empty(), "{}: empty origin", s.id);
            assert!(!s.display_name.is_empty(), "{}: empty display_name", s.id);
            assert!(!s.family.is_empty(), "{}: empty family", s.id);
            // origin_repo is an "org/name" slug the picker turns into an HF URL.
            assert_eq!(
                s.origin_repo.split('/').count(),
                2,
                "{}: origin_repo is not org/name: {}",
                s.id,
                s.origin_repo
            );
        }
    }

    #[test]
    fn mmproj_fields_are_internally_consistent() {
        for s in STARTERS {
            // The mmproj file, its digest, and a non-zero byte count travel
            // together, and a vision entry ships a projector while a text entry
            // does not (llama.cpp needs the mmproj to see images).
            assert_eq!(
                s.mmproj_file.is_some(),
                s.mmproj_sha256.is_some(),
                "{}: mmproj file/sha presence mismatch",
                s.id
            );
            assert_eq!(
                s.mmproj_file.is_some(),
                s.mmproj_bytes > 0,
                "{}: mmproj file/bytes presence mismatch",
                s.id
            );
            assert_eq!(
                s.vision,
                s.mmproj_file.is_some(),
                "{}: vision/mmproj mismatch",
                s.id
            );
        }
    }

    #[test]
    fn reasoning_always_entries_also_emit_thinking() {
        // A model whose reasoning cannot be turned off must also be flagged as
        // a thinking model, or the picker badge and `/think` gate disagree.
        for s in STARTERS {
            if s.reasoning_always {
                assert!(s.thinking, "{}: reasoning_always implies thinking", s.id);
            }
        }
    }

    #[test]
    fn every_entry_has_a_positive_runtime_estimate() {
        for s in STARTERS {
            assert!(
                s.est_runtime_gb > 0.0,
                "{}: non-positive est_runtime_gb",
                s.id
            );
        }
    }

    #[test]
    fn family_per_tier() {
        // Each entry carries a non-empty family label.
        assert_eq!(starter(Tier::Fast).family, "Qwen");
        assert_eq!(starter(Tier::Balanced).family, "Gemma");
        assert_eq!(starter(Tier::Smartest).family, "gpt-oss");
        for s in STARTERS {
            assert!(!s.family.is_empty(), "{}: family is empty", s.repo);
        }
    }

    #[test]
    fn category_per_tier() {
        // The Discover staff-picks list groups starters into use-case sections,
        // so every entry carries a non-empty category label.
        assert_eq!(starter(Tier::Fast).category, "Everyday chat");
        assert_eq!(starter(Tier::Balanced).category, "Everyday chat");
        assert_eq!(starter(Tier::Smartest).category, "Deep reasoning");
        for s in STARTERS {
            assert!(!s.category.is_empty(), "{}: category is empty", s.repo);
        }
    }

    #[test]
    fn vision_and_mmproj_per_tier() {
        // Fast (Qwen3.5) and Balanced (Gemma 4) are multimodal and each carries
        // a vision projector; Smartest (gpt-oss) is text-only, so it has no
        // mmproj companion at all.
        for tier in [Tier::Fast, Tier::Balanced] {
            let s = starter(tier);
            assert!(s.vision, "{tier:?} should be a vision tier");
            assert!(s.mmproj_file.is_some());
            assert!(s.mmproj_sha256.is_some());
            assert!(s.mmproj_bytes > 0);
        }
        let smartest = starter(Tier::Smartest);
        assert!(!smartest.vision);
        assert!(smartest.mmproj_file.is_none());
        assert!(smartest.mmproj_sha256.is_none());
        assert_eq!(smartest.mmproj_bytes, 0);
    }

    /// The `thinking` flag is the passive "this model reasons" badge: it drives
    /// the picker tag, the `/think` capability gate, and the earlier-turn
    /// reasoning strip. It must match each curated model's real behavior, or a
    /// reasoning model is wrongly told it "does not emit thinking tokens".
    /// Qwen3.5, Gemma 4, and gpt-oss all reason (Gemma 4 via its optional
    /// `enable_thinking` chat-template toggle).
    #[test]
    fn thinking_flag_per_tier() {
        assert!(starter(Tier::Fast).thinking, "Qwen3.5 reasons");
        assert!(
            starter(Tier::Balanced).thinking,
            "Gemma 4 reasons on demand"
        );
        assert!(starter(Tier::Smartest).thinking, "gpt-oss reasons");
    }

    /// `reasoning_always` marks models whose reasoning cannot be turned off.
    /// Only gpt-oss (Harmony) is structurally always-on; Qwen3.5's reasoning is
    /// optional (off by default via the kwarg blast) and Gemma does not reason.
    #[test]
    fn reasoning_always_flag_per_tier() {
        assert!(
            starter(Tier::Smartest).reasoning_always,
            "gpt-oss always reasons"
        );
        assert!(
            !starter(Tier::Fast).reasoning_always,
            "Qwen3.5 reasoning is optional"
        );
        assert!(
            !starter(Tier::Balanced).reasoning_always,
            "Gemma does not force reasoning"
        );
    }

    #[test]
    fn all_revisions_are_40_hex() {
        for s in STARTERS {
            assert!(
                is_lower_hex(s.revision, 40),
                "{}: revision is not 40-hex: {}",
                s.repo,
                s.revision
            );
        }
    }

    #[test]
    fn all_sha256_are_64_hex() {
        for s in STARTERS {
            assert!(
                is_lower_hex(s.sha256, 64),
                "{}: weights sha256 is not lowercase 64-hex: {}",
                s.repo,
                s.sha256
            );
            if let Some(mm) = s.mmproj_sha256 {
                assert!(
                    is_lower_hex(mm, 64),
                    "{}: mmproj sha256 is not lowercase 64-hex: {mm}",
                    s.repo
                );
            }
        }
    }

    #[test]
    fn license_notes_per_tier() {
        // The picker surfaces these verbatim. Every tier ships under a
        // permissive license: Qwen3.5, Gemma 4, and gpt-oss are all Apache 2.0.
        assert_eq!(starter(Tier::Fast).license_note, "Apache 2.0");
        assert_eq!(starter(Tier::Balanced).license_note, "Apache 2.0");
        assert_eq!(starter(Tier::Smartest).license_note, "Apache 2.0");
    }

    #[test]
    fn origin_per_tier() {
        // The picker's Origin row links to each maker's own official HF page
        // for verification; the maker can differ from the GGUF download repo.
        let cases = [
            (Tier::Fast, "Alibaba", "Qwen/Qwen3.5-9B"),
            (Tier::Balanced, "Google", "google/gemma-4-12B-it"),
            (Tier::Smartest, "OpenAI", "openai/gpt-oss-20b"),
        ];
        for (tier, origin, origin_repo) in cases {
            let s = starter(tier);
            assert_eq!(s.origin, origin);
            assert_eq!(s.origin_repo, origin_repo);
            // origin_repo is an "org/name" slug the picker turns into an HF URL.
            assert_eq!(s.origin_repo.split('/').count(), 2);
            assert!(!s.origin.is_empty());
        }
    }

    #[test]
    fn mmproj_hashes_are_distinct_between_vision_tiers() {
        let fast = starter(Tier::Fast);
        let balanced = starter(Tier::Balanced);
        // The two vision tiers (Qwen3.5 and Gemma 4) ship their own mmproj; the
        // sizes and hashes must differ, or a copy/paste swap slipped in.
        assert_ne!(fast.mmproj_bytes, balanced.mmproj_bytes);
        assert_ne!(fast.mmproj_sha256.unwrap(), balanced.mmproj_sha256.unwrap());
    }

    #[test]
    fn fit_cutoffs() {
        const GIB: u64 = 1 << 30;
        // (ram_gib, expected fit for Fast 8.5 / Balanced 9.5 / Smartest 13.3)
        let table: &[(u64, [RamFit; 3])] = &[
            (8, [RamFit::TooBig, RamFit::TooBig, RamFit::TooBig]),
            (16, [RamFit::Fits, RamFit::Fits, RamFit::Tight]),
            (24, [RamFit::Fits, RamFit::Fits, RamFit::Fits]),
            (32, [RamFit::Fits, RamFit::Fits, RamFit::Fits]),
        ];
        for (ram_gib, expected) in table {
            for (s, want) in onboarding_heroes().iter().zip(expected) {
                let got = ram_fit(s.est_runtime_gb, ram_gib * GIB);
                assert_eq!(
                    got, *want,
                    "{} at {ram_gib} GiB: expected {want:?}, got {got:?}",
                    s.display_name
                );
            }
        }
    }

    #[test]
    fn download_specs_includes_mmproj() {
        let fast = starter(Tier::Fast);
        let specs = download_specs(fast);
        assert_eq!(specs.len(), 2);
        assert_eq!(
            specs[0].url,
            format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                fast.repo, fast.revision, fast.file_name
            )
        );
        assert_eq!(specs[0].file, fast.file_name);
        assert_eq!(specs[0].sha256, fast.sha256);
        assert_eq!(specs[0].total_bytes, fast.size_bytes);
        assert_eq!(
            specs[1].url,
            format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                fast.repo,
                fast.revision,
                fast.mmproj_file.unwrap()
            )
        );
        assert_eq!(specs[1].file, fast.mmproj_file.unwrap());
        assert_eq!(specs[1].sha256, fast.mmproj_sha256.unwrap());
        assert_eq!(specs[1].total_bytes, fast.mmproj_bytes);

        let smartest = starter(Tier::Smartest);
        let specs = download_specs(smartest);
        assert_eq!(specs.len(), 1);
        assert_eq!(
            specs[0].url,
            format!(
                "https://huggingface.co/{}/resolve/{}/{}",
                smartest.repo, smartest.revision, smartest.file_name
            )
        );
    }

    #[test]
    fn installed_model_id_is_repo_colon_file() {
        // The manifest-row id is "<repo>:<file_name>"; `installed_model_id` is
        // its single source of truth, so `to_installed_model` never drifts from
        // the installed-state probe.
        let s = &STARTERS[0];
        assert_eq!(installed_model_id(s), format!("{}:{}", s.repo, s.file_name));
        assert_eq!(to_installed_model(s).id, installed_model_id(s));
    }

    #[test]
    fn to_installed_model_maps_fields() {
        let balanced = starter(Tier::Balanced);
        let m = to_installed_model(balanced);
        assert_eq!(m.id, format!("{}:{}", balanced.repo, balanced.file_name));
        assert_eq!(m.display_name, balanced.display_name);
        assert_eq!(m.repo, balanced.repo);
        assert_eq!(m.revision, balanced.revision);
        assert_eq!(m.file_name, balanced.file_name);
        assert_eq!(m.sha256, balanced.sha256);
        assert_eq!(m.size_bytes, balanced.size_bytes);
        assert_eq!(m.quant, balanced.quant);
        assert_eq!(m.vision, balanced.vision);
        assert_eq!(m.thinking, balanced.thinking);
        assert_eq!(m.reasoning_always, balanced.reasoning_always);
        assert_eq!(m.mmproj_file.as_deref(), balanced.mmproj_file);
        assert_eq!(m.mmproj_sha256.as_deref(), balanced.mmproj_sha256);

        // Text-only starter: the Option fields map to None.
        let smartest = starter(Tier::Smartest);
        let m = to_installed_model(smartest);
        assert_eq!(m.id, format!("{}:{}", smartest.repo, smartest.file_name));
        assert_eq!(m.mmproj_file, None);
        assert_eq!(m.mmproj_sha256, None);
    }
}
