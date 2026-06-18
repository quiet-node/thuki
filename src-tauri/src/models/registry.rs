/*!
 * Curated starter model registry for the built-in llama.cpp engine.
 *
 * Three tiers (Fast / Balanced / Smartest) cover the RAM spectrum of Apple
 * Silicon Macs. Every entry pins a Hugging Face repo at an exact git revision
 * and carries the SHA-256 of each blob, so a starter download is reproducible
 * and verifiable end to end (the digests feed straight into
 * [`crate::models::download::DownloadSpec`] which verifies them on install).
 *
 * Hashes and sizes were read from the Hugging Face tree-at-revision API
 * (`/api/models/<repo>/tree/<revision>`) on 2026-06-17, so each digest
 * matches the pinned commit, not whatever `main` later points to.
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
    /// Which speed/quality tier this entry fills.
    pub tier: Tier,
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
        tier: Tier::Fast,
        display_name: "Qwen3.5 9B",
        repo: "unsloth/Qwen3.5-9B-GGUF",
        revision: "3885219b6810b007914f3a7950a8d1b469d598a5",
        file_name: "Qwen3.5-9B-Q4_K_M.gguf",
        sha256: "03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8",
        size_bytes: 5_680_522_464,
        quant: "Q4_K_M",
        vision: true,
        thinking: false,
        mmproj_file: Some("mmproj-BF16.gguf"),
        mmproj_sha256: Some("853698ce7aa6c7ba732478bad280240969ddf7b0fcbf93900046f63903a83383"),
        mmproj_bytes: 921_705_024,
        est_runtime_gb: 8.5,
        license_note: "Apache 2.0",
        origin: "Alibaba",
        origin_repo: "Qwen/Qwen3.5-9B",
    },
    Starter {
        tier: Tier::Balanced,
        display_name: "Gemma 4 12B",
        repo: "google/gemma-4-12B-it-qat-q4_0-gguf",
        revision: "f6e7774e6148da3b7f201e42ba37cf084c1db35f",
        file_name: "gemma-4-12b-it-qat-q4_0.gguf",
        sha256: "faff1a63667fac17ac5e777f47114688fcefea96e220e211aaa8d62c2c4561f1",
        size_bytes: 6_975_877_728,
        quant: "Q4_0",
        vision: true,
        thinking: false,
        mmproj_file: Some("mmproj-gemma-4-12b-it-qat-q4_0.gguf"),
        mmproj_sha256: Some("e70b0e5cd80323d5d588b4ed06780356b7b1ba03995a4b8164c6ae9db0ff5989"),
        mmproj_bytes: 175_115_264,
        est_runtime_gb: 9.5,
        license_note: "Apache 2.0",
        origin: "Google",
        origin_repo: "google/gemma-4-12B-it",
    },
    Starter {
        tier: Tier::Smartest,
        display_name: "gpt-oss 20B",
        repo: "ggml-org/gpt-oss-20b-GGUF",
        revision: "e1dc459feff949ff451ce107337a2026daa80df8",
        file_name: "gpt-oss-20b-mxfp4.gguf",
        sha256: "be37a636aca0fc1aae0d32325f82f6b4d21495f06823b5fbc1898ae0303e9935",
        size_bytes: 12_109_566_560,
        quant: "MXFP4",
        vision: false,
        thinking: false,
        mmproj_file: None,
        mmproj_sha256: None,
        mmproj_bytes: 0,
        est_runtime_gb: 13.3,
        license_note: "Apache 2.0",
        origin: "OpenAI",
        origin_repo: "openai/gpt-oss-20b",
    },
];

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

/// Manifest row for an installed starter. id = `"<repo>:<file_name>"`.
pub fn to_installed_model(s: &Starter) -> InstalledModel {
    InstalledModel {
        id: format!("{}:{}", s.repo, s.file_name),
        display_name: s.display_name.to_string(),
        repo: s.repo.to_string(),
        revision: s.revision.to_string(),
        file_name: s.file_name.to_string(),
        sha256: s.sha256.to_string(),
        size_bytes: s.size_bytes,
        quant: s.quant.to_string(),
        vision: s.vision,
        thinking: s.thinking,
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

    fn starter(tier: Tier) -> &'static Starter {
        STARTERS.iter().find(|s| s.tier == tier).unwrap()
    }

    #[test]
    fn three_tiers_present() {
        assert_eq!(STARTERS.len(), 3);
        assert_eq!(
            STARTERS.iter().map(|s| s.tier).collect::<Vec<_>>(),
            vec![Tier::Fast, Tier::Balanced, Tier::Smartest]
        );
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
            for (s, want) in STARTERS.iter().zip(expected) {
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
