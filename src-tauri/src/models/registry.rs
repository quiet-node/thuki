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
 * (`/api/models/<repo>/tree/<revision>`) on 2026-06-10, so each digest
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
    /// Human-readable label shown in the picker (e.g. "Gemma 3 4B").
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
}

/// The curated starters, ordered Fast, Balanced, Smartest.
pub const STARTERS: &[Starter] = &[
    Starter {
        tier: Tier::Fast,
        display_name: "Gemma 3 4B",
        repo: "ggml-org/gemma-3-4b-it-GGUF",
        revision: "d0976223747697cb51e056d85c532013931fe52e",
        file_name: "gemma-3-4b-it-Q4_K_M.gguf",
        sha256: "882e8d2db44dc554fb0ea5077cb7e4bc49e7342a1f0da57901c0802ea21a0863",
        size_bytes: 2_489_757_856,
        quant: "Q4_K_M",
        vision: true,
        thinking: false,
        mmproj_file: Some("mmproj-model-f16.gguf"),
        mmproj_sha256: Some("8c0fb064b019a6972856aaae2c7e4792858af3ca4561be2dbf649123ba6c40cb"),
        mmproj_bytes: 851_251_104,
        est_runtime_gb: 5.0,
        license_note: "Gemma Terms of Use",
    },
    Starter {
        tier: Tier::Balanced,
        display_name: "Gemma 3 12B",
        repo: "ggml-org/gemma-3-12b-it-GGUF",
        revision: "ec0cbabd8dbff316f659876a50202295c3c4a314",
        file_name: "gemma-3-12b-it-Q4_K_M.gguf",
        sha256: "7bb69bff3f48a7b642355d64a90e481182a7794707b3133890646b1efa778ff5",
        size_bytes: 7_300_574_976,
        quant: "Q4_K_M",
        vision: true,
        thinking: false,
        mmproj_file: Some("mmproj-model-f16.gguf"),
        mmproj_sha256: Some("30c02d056410848227001830866e0a269fcc28aaf8ca971bded494003de9f5a5"),
        mmproj_bytes: 854_200_224,
        est_runtime_gb: 11.5,
        license_note: "Gemma Terms of Use",
    },
    Starter {
        tier: Tier::Smartest,
        display_name: "Phi-4 14B",
        repo: "bartowski/phi-4-GGUF",
        revision: "19cd65f97c2f1712a81c506611d3f9c94b16a1e1",
        file_name: "phi-4-Q4_K_M.gguf",
        sha256: "009aba717c09d4a35890c7d35eb59d54e1dba884c7c526e7197d9c13ab5911d9",
        size_bytes: 9_053_114_816,
        quant: "Q4_K_M",
        vision: false,
        thinking: false,
        mmproj_file: None,
        mmproj_sha256: None,
        mmproj_bytes: 0,
        est_runtime_gb: 10.7,
        license_note: "MIT",
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
    fn balanced_is_vision() {
        let balanced = starter(Tier::Balanced);
        assert!(balanced.vision);
        assert!(balanced.mmproj_file.is_some());
        assert!(balanced.mmproj_sha256.is_some());
        assert!(balanced.mmproj_bytes > 0);
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
    fn mmproj_hashes_are_distinct_between_gemma_tiers() {
        let fast = starter(Tier::Fast);
        let balanced = starter(Tier::Balanced);
        // Both Gemma mmproj files share a name but differ in size, so their
        // hashes must differ; identical hashes would mean a swap happened.
        assert_ne!(fast.mmproj_bytes, balanced.mmproj_bytes);
        assert_ne!(fast.mmproj_sha256.unwrap(), balanced.mmproj_sha256.unwrap());
    }

    #[test]
    fn fit_cutoffs() {
        const GIB: u64 = 1 << 30;
        // (ram_gib, expected fit for Fast 5.0 / Balanced 11.5 / Smartest 10.7)
        let table: &[(u64, [RamFit; 3])] = &[
            (8, [RamFit::Tight, RamFit::TooBig, RamFit::TooBig]),
            (16, [RamFit::Fits, RamFit::Tight, RamFit::Tight]),
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
