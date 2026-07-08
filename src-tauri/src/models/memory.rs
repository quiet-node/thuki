//! Live-memory admission control for loading a model into the built-in engine.
//!
//! Issue #296: on a memory-constrained Mac, Thuki auto-loaded a large model
//! with no free-memory check, contributing to a whole-machine freeze. This
//! module adds a preflight that estimates a model's resident footprint, reads
//! how much system memory is actually available right now, and produces a
//! fit verdict the load paths gate on.
//!
//! The gate is deliberately *forgiving*: the footprint estimate is inherently
//! approximate (weights-on-disk plus a fixed overhead, ignoring the mmproj
//! blob and the context-scaled KV cache, mirroring the Library/Discover fit
//! hint), and real-world estimators are documented to be off by up to ~2x. So
//! it only ever hard-blocks a load whose estimate lands clearly above a
//! ceiling of available memory, and a user-facing `force` always bypasses it.
//!
//! ## Why "available", not "total"
//!
//! `super::system_ram_bytes` reads `hw.memsize`, the machine's *total* physical
//! RAM. That ignores everything already resident (the OS, other apps, a model
//! already loaded), so it cannot answer "will this fit right now". The reader
//! here uses the mach VM statistics to sum the pages the kernel can hand out
//! without swapping: free, inactive, speculative, and purgeable.
//!
//! ## Thread-safety
//!
//! [`available_memory_bytes`] is a stateless mach syscall that reads a
//! kernel-owned snapshot; it holds no shared state and is safe to call
//! concurrently. The read is a point-in-time sample: on a busy machine the
//! value can move between the read and the load, which is acceptable for an
//! advisory, force-overridable gate.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::manifest::InstalledModel;
use crate::config::defaults::{
    MODEL_FIT_CEILING_FRACTION, MODEL_FIT_COMFORT_FRACTION, RUNTIME_OVERHEAD_GB,
};

/// Bytes in one gibibyte (2^30), the unit `RUNTIME_OVERHEAD_GB` is expressed in.
const BYTES_PER_GIB: u64 = 1 << 30;

/// How a model's estimated footprint fits the memory available right now.
///
/// Serialized to the frontend (snake_case) so the deferred "may not fit"
/// override UI can render the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryFit {
    /// Fits with healthy headroom.
    Comfortable,
    /// Fits but close to the ceiling; a soft warning, never a block.
    Tight,
    /// The estimate exceeds the ceiling fraction of available memory.
    Insufficient,
}

/// The admission decision for a specific pending load.
///
/// Internal to the load paths; the numbers on `Block` build the user-facing
/// error copy and back the unit tests. The machine-readable footprint the
/// override UI renders comes from the [`estimate_model_fit`] command, which is
/// the single numeric source of truth for the frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryGate {
    /// Load may proceed.
    Proceed,
    /// Load is blocked; the model's estimate does not fit available memory.
    Block {
        /// Estimated resident footprint of the model, in bytes.
        required_bytes: u64,
        /// Memory judged available for the load, in bytes (already crediting a
        /// resident model that would be evicted first).
        available_bytes: u64,
    },
}

/// Available system memory in bytes from raw mach VM page counts and the page
/// size. Pure arithmetic split out from the [`available_memory_bytes`] syscall
/// so the byte math is unit-tested. Every step saturates so a pathological
/// kernel snapshot can never overflow into a small (falsely "insufficient")
/// number.
pub fn available_from_vm_stats(
    free_pages: u64,
    inactive_pages: u64,
    speculative_pages: u64,
    purgeable_pages: u64,
    page_size: u64,
) -> u64 {
    free_pages
        .saturating_add(inactive_pages)
        .saturating_add(speculative_pages)
        .saturating_add(purgeable_pages)
        .saturating_mul(page_size)
}

/// Estimated resident footprint in bytes for a model whose weights occupy
/// `weights_bytes` on disk: the weights plus the fixed [`RUNTIME_OVERHEAD_GB`]
/// (KV cache at the default context plus runtime buffers). This is the
/// byte-domain twin of `super::estimate_runtime_gb_from_bytes`, kept on the
/// same weights-plus-overhead convention so the gate and the fit hint agree.
/// Saturating so a corrupt manifest size cannot overflow.
pub fn estimate_required_bytes(weights_bytes: u64) -> u64 {
    // f64 -> u64 cast saturates at u64::MAX and floors negatives to 0; the
    // operand is a small positive constant so neither edge is reachable here.
    let overhead = (RUNTIME_OVERHEAD_GB * BYTES_PER_GIB as f64) as u64;
    weights_bytes.saturating_add(overhead)
}

/// Fit verdict for `required_bytes` against `available_bytes`.
///
/// The ceiling models Jan's published guidance that a model should stay under
/// ~80% of available memory ([`MODEL_FIT_CEILING_FRACTION`]); below the
/// comfort fraction ([`MODEL_FIT_COMFORT_FRACTION`]) there is healthy headroom.
/// `available_bytes == 0` means the reader failed (or a broken snapshot): the
/// gate must not brick loads on a bad read, so it reports `Comfortable`.
pub fn assess_fit(required_bytes: u64, available_bytes: u64) -> MemoryFit {
    if available_bytes == 0 {
        return MemoryFit::Comfortable;
    }
    let required = required_bytes as f64;
    let available = available_bytes as f64;
    if required > MODEL_FIT_CEILING_FRACTION * available {
        MemoryFit::Insufficient
    } else if required > MODEL_FIT_COMFORT_FRACTION * available {
        MemoryFit::Tight
    } else {
        MemoryFit::Comfortable
    }
}

/// Total on-disk weights bytes for an installed model: `size_bytes` for a
/// single-file model, or the sum of every shard for a split model (whose
/// `size_bytes` records only the first shard). Saturating on the sum.
pub fn model_weights_bytes(model: &InstalledModel) -> u64 {
    if model.parts.is_empty() {
        model.size_bytes
    } else {
        model
            .parts
            .iter()
            .fold(0u64, |acc, part| acc.saturating_add(part.size_bytes))
    }
}

/// Weights bytes of the installed row whose weights blob path equals
/// `resident_path`, or 0 when none matches. Lets the gate credit a resident
/// model's footprint back into available memory, since the engine evicts the
/// current model before loading the next one.
///
/// A split model loads through a symlink shim, so its resident path is not a
/// blob path and will not match here; that yields 0 credit, which only makes
/// the gate more conservative (never a false pass), and `force` covers the
/// rare "switch away from a split model" corner.
pub fn resident_weights_bytes(resident_path: &Path, installed: &[(u64, PathBuf)]) -> u64 {
    installed
        .iter()
        .find(|(_, path)| path.as_path() == resident_path)
        .map_or(0, |(bytes, _)| *bytes)
}

/// The admission decision for loading the model at `target_path` (weights
/// `target_weights_bytes`) given the memory available right now.
///
/// - `forced` (the user's "load anyway") short-circuits to [`MemoryGate::Proceed`].
/// - When the engine is already serving this exact path (`resident_path ==
///   target_path`), the ensure is a no-op with no new allocation, so it
///   proceeds without any arithmetic: a model that already fills memory must
///   never be blocked from continuing to serve.
/// - When a *different* model is resident, its footprint is credited back into
///   available memory because it is evicted before the new load.
/// - Otherwise the model's estimate is judged against available memory and
///   blocked only on an [`MemoryFit::Insufficient`] verdict.
pub fn evaluate_load_gate(
    target_weights_bytes: u64,
    available_bytes: u64,
    resident_path: Option<&Path>,
    target_path: &Path,
    installed: &[(u64, PathBuf)],
    forced: bool,
) -> MemoryGate {
    if forced {
        return MemoryGate::Proceed;
    }
    let credit = match resident_path {
        // Already serving the exact model: reused in place, nothing to admit.
        Some(path) if path == target_path => return MemoryGate::Proceed,
        // A different model is resident and will be evicted first; its memory
        // returns to the pool before the new load allocates.
        Some(path) => resident_weights_bytes(path, installed),
        None => 0,
    };
    let required_bytes = estimate_required_bytes(target_weights_bytes);
    let effective_available = available_bytes.saturating_add(credit);
    match assess_fit(required_bytes, effective_available) {
        MemoryFit::Insufficient => MemoryGate::Block {
            required_bytes,
            available_bytes: effective_available,
        },
        MemoryFit::Comfortable | MemoryFit::Tight => MemoryGate::Proceed,
    }
}

/// Live available system memory in bytes via mach `host_statistics64`
/// (`HOST_VM_INFO64`): the sum of the free, inactive, speculative, and
/// purgeable pages, which the kernel can reclaim without swapping. Returns 0
/// on any syscall failure, which the fit estimator treats as "unknown" and
/// never blocks on.
///
/// Not covered by the cargo coverage gate: a direct OS syscall whose only
/// logic is the pure [`available_from_vm_stats`] arithmetic it delegates to
/// (mirrors `super::system_ram_bytes` and `storage::free_disk_bytes`).
#[cfg_attr(coverage_nightly, coverage(off))]
// `libc::mach_host_self` is deprecated in favor of the `mach2` crate, but it is
// a stable one-call syscall and pulling in another dependency for it is not
// worth it; the call is confined to this wrapper.
#[allow(deprecated)]
pub fn available_memory_bytes() -> u64 {
    // SAFETY: `mach_host_self` returns the host port. `page_size` and `stats`
    // are valid, correctly-sized stack buffers; `count` is initialized to the
    // element count `host_statistics64` expects for `HOST_VM_INFO64` and is
    // read/written in place. Every non-success return is mapped to 0, so no
    // uninitialized data is ever consumed.
    unsafe {
        let host = libc::mach_host_self();
        let page_size = libc::sysconf(libc::_SC_PAGESIZE);
        if page_size <= 0 {
            return 0;
        }
        let mut stats: libc::vm_statistics64 = std::mem::zeroed();
        let mut count = (std::mem::size_of::<libc::vm_statistics64>()
            / std::mem::size_of::<libc::integer_t>())
            as libc::mach_msg_type_number_t;
        let kr = libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            &mut stats as *mut libc::vm_statistics64 as libc::host_info64_t,
            &mut count,
        );
        if kr != libc::KERN_SUCCESS {
            return 0;
        }
        available_from_vm_stats(
            stats.free_count as u64,
            stats.inactive_count as u64,
            stats.speculative_count as u64,
            stats.purgeable_count as u64,
            page_size as u64,
        )
    }
}

/// The fit estimate the frontend renders and drives the override UI from:
/// the model's estimated footprint, live available memory, and the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ModelFitEstimate {
    /// Estimated resident footprint of the model, in bytes.
    pub required_bytes: u64,
    /// Live available system memory, in bytes.
    pub available_bytes: u64,
    /// The fit verdict for the two figures above.
    pub verdict: MemoryFit,
}

/// Pure core of [`estimate_model_fit`]: given a model's weights bytes and the
/// live available memory, assemble the [`ModelFitEstimate`]. Split from the
/// command so the assembly is unit-tested without a Tauri runtime.
pub fn build_model_fit_estimate(weights_bytes: u64, available_bytes: u64) -> ModelFitEstimate {
    let required_bytes = estimate_required_bytes(weights_bytes);
    ModelFitEstimate {
        required_bytes,
        available_bytes,
        verdict: assess_fit(required_bytes, available_bytes),
    }
}

/// Estimates whether a model fits in the memory available right now, for the
/// frontend's "may not fit, load anyway?" affordance. `model_id` names the
/// installed model, or the active provider's model when omitted.
///
/// Thin Tauri wrapper (coverage-off): resolves the model id, reads its weights
/// size from the manifest, and delegates every decision to the unit-tested
/// [`build_model_fit_estimate`] and [`available_memory_bytes`].
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn estimate_model_fit(
    model_id: Option<String>,
    db: tauri::State<'_, crate::history::Database>,
    config: tauri::State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
) -> Result<ModelFitEstimate, String> {
    let model_id = model_id.unwrap_or_else(|| {
        config
            .read()
            .inference
            .active_provider_model()
            .to_string()
    });
    if model_id.is_empty() {
        return Err("No model selected.".to_string());
    }
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let row = super::manifest::get(&conn, &model_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "The selected model is not installed.".to_string())?;
    let weights_bytes = model_weights_bytes(&row);
    Ok(build_model_fit_estimate(
        weights_bytes,
        available_memory_bytes(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::HfGgufPart;

    /// Builds a minimal installed row carrying only the size fields the memory
    /// math reads; every other field is a harmless placeholder.
    fn row(size_bytes: u64, parts: Vec<HfGgufPart>) -> InstalledModel {
        InstalledModel {
            id: "repo:file.gguf".to_string(),
            display_name: "Test".to_string(),
            repo: "repo".to_string(),
            revision: "rev".to_string(),
            file_name: "file.gguf".to_string(),
            sha256: "sha".to_string(),
            size_bytes,
            quant: "Q4_K_M".to_string(),
            vision: false,
            thinking: false,
            reasoning_always: false,
            mmproj_file: None,
            mmproj_sha256: None,
            parts,
        }
    }

    /// Builds a shard with a given size; the other fields are placeholders.
    fn part(sha: &str, size_bytes: u64) -> HfGgufPart {
        HfGgufPart {
            file: format!("{sha}.gguf"),
            sha256: sha.to_string(),
            size_bytes,
        }
    }

    #[test]
    fn available_from_vm_stats_sums_pages_times_page_size() {
        // (1 + 2 + 3 + 4) pages * 4096 bytes.
        assert_eq!(available_from_vm_stats(1, 2, 3, 4, 4096), 10 * 4096);
    }

    #[test]
    fn available_from_vm_stats_saturates_instead_of_overflowing() {
        // A pathological snapshot must never wrap into a small number.
        assert_eq!(
            available_from_vm_stats(u64::MAX, u64::MAX, 0, 0, 2),
            u64::MAX
        );
        assert_eq!(available_from_vm_stats(u64::MAX, 0, 0, 0, u64::MAX), u64::MAX);
    }

    #[test]
    fn estimate_required_bytes_adds_fixed_overhead() {
        let overhead = (RUNTIME_OVERHEAD_GB * BYTES_PER_GIB as f64) as u64;
        assert_eq!(estimate_required_bytes(0), overhead);
        assert_eq!(estimate_required_bytes(5 * BYTES_PER_GIB), 5 * BYTES_PER_GIB + overhead);
    }

    #[test]
    fn estimate_required_bytes_saturates() {
        assert_eq!(estimate_required_bytes(u64::MAX), u64::MAX);
    }

    #[test]
    fn assess_fit_unknown_available_is_comfortable() {
        // A failed reader (0) must never block a load.
        assert_eq!(assess_fit(u64::MAX, 0), MemoryFit::Comfortable);
    }

    #[test]
    fn assess_fit_classifies_against_fractions() {
        let available = 10 * BYTES_PER_GIB;
        // Comfortable: at/under the comfort fraction (0.60 -> 6 GiB).
        assert_eq!(assess_fit(5 * BYTES_PER_GIB, available), MemoryFit::Comfortable);
        assert_eq!(
            assess_fit((MODEL_FIT_COMFORT_FRACTION * available as f64) as u64, available),
            MemoryFit::Comfortable
        );
        // Tight: above comfort, at/under the ceiling (0.80 -> 8 GiB).
        assert_eq!(assess_fit(7 * BYTES_PER_GIB, available), MemoryFit::Tight);
        assert_eq!(
            assess_fit((MODEL_FIT_CEILING_FRACTION * available as f64) as u64, available),
            MemoryFit::Tight
        );
        // Insufficient: clearly above the ceiling.
        assert_eq!(assess_fit(9 * BYTES_PER_GIB, available), MemoryFit::Insufficient);
    }

    #[test]
    fn model_weights_bytes_single_file_uses_size() {
        assert_eq!(model_weights_bytes(&row(1234, vec![])), 1234);
    }

    #[test]
    fn model_weights_bytes_split_sums_all_shards() {
        let model = row(100, vec![part("a", 100), part("b", 250), part("c", 50)]);
        // Split sums every shard, not the first-shard `size_bytes` (100).
        assert_eq!(model_weights_bytes(&model), 400);
    }

    #[test]
    fn resident_weights_bytes_matches_by_path() {
        let installed = vec![
            (10u64, PathBuf::from("/blobs/a")),
            (20u64, PathBuf::from("/blobs/b")),
        ];
        assert_eq!(resident_weights_bytes(Path::new("/blobs/b"), &installed), 20);
    }

    #[test]
    fn resident_weights_bytes_no_match_is_zero() {
        let installed = vec![(10u64, PathBuf::from("/blobs/a"))];
        // An unknown path (e.g. a split-shim symlink) credits nothing.
        assert_eq!(resident_weights_bytes(Path::new("/blobs/shim"), &installed), 0);
    }

    #[test]
    fn gate_forced_always_proceeds() {
        // Force bypasses even a clearly-oversized model against no memory.
        let gate = evaluate_load_gate(
            u64::MAX,
            1,
            None,
            Path::new("/blobs/target"),
            &[],
            true,
        );
        assert_eq!(gate, MemoryGate::Proceed);
    }

    #[test]
    fn gate_blocks_oversized_cold_load() {
        // 20 GiB model, 10 GiB available, nothing resident -> Insufficient.
        let gate = evaluate_load_gate(
            20 * BYTES_PER_GIB,
            10 * BYTES_PER_GIB,
            None,
            Path::new("/blobs/target"),
            &[],
            false,
        );
        match gate {
            MemoryGate::Block {
                required_bytes,
                available_bytes,
            } => {
                assert_eq!(required_bytes, estimate_required_bytes(20 * BYTES_PER_GIB));
                assert_eq!(available_bytes, 10 * BYTES_PER_GIB);
            }
            MemoryGate::Proceed => panic!("expected Block"),
        }
    }

    #[test]
    fn gate_allows_fitting_cold_load() {
        // 4 GiB model, 24 GiB available -> Comfortable -> Proceed.
        let gate = evaluate_load_gate(
            4 * BYTES_PER_GIB,
            24 * BYTES_PER_GIB,
            None,
            Path::new("/blobs/target"),
            &[],
            false,
        );
        assert_eq!(gate, MemoryGate::Proceed);
    }

    #[test]
    fn gate_same_model_resident_proceeds_without_arithmetic() {
        // The exact model is already resident and fills memory (available ~0);
        // it must never be blocked from continuing to serve.
        let target = PathBuf::from("/blobs/target");
        let gate = evaluate_load_gate(
            20 * BYTES_PER_GIB,
            0,
            Some(&target),
            &target,
            &[],
            false,
        );
        assert_eq!(gate, MemoryGate::Proceed);
    }

    #[test]
    fn gate_credits_resident_model_on_switch() {
        // Switch A(14 GiB, resident) -> B(8 GiB). Live available reads only
        // 1 GiB because A fills memory; without crediting A, B (~10 GiB with
        // overhead) is blocked. A is evicted first, so B is judged against
        // 1 + 14 = 15 GiB and fits. The credit is load-bearing here.
        let a = PathBuf::from("/blobs/a");
        let b = PathBuf::from("/blobs/b");
        let installed = vec![(14 * BYTES_PER_GIB, a.clone())];
        // Without the credit this exact model would be blocked.
        assert!(matches!(
            evaluate_load_gate(8 * BYTES_PER_GIB, BYTES_PER_GIB, None, &b, &installed, false),
            MemoryGate::Block { .. }
        ));
        // With A resident and credited back, it proceeds.
        let gate = evaluate_load_gate(
            8 * BYTES_PER_GIB,
            BYTES_PER_GIB,
            Some(&a),
            &b,
            &installed,
            false,
        );
        assert_eq!(gate, MemoryGate::Proceed);
    }

    #[test]
    fn gate_blocks_switch_to_oversized_even_with_credit() {
        // Switch A(4 GiB, resident) -> B(30 GiB) on a 10 GiB machine: crediting
        // A's 4 GiB back gives 6 GiB available for a ~32 GiB model -> blocked.
        let a = PathBuf::from("/blobs/a");
        let b = PathBuf::from("/blobs/b");
        let installed = vec![(4 * BYTES_PER_GIB, a.clone())];
        let gate = evaluate_load_gate(
            30 * BYTES_PER_GIB,
            2 * BYTES_PER_GIB,
            Some(&a),
            &b,
            &installed,
            false,
        );
        match gate {
            MemoryGate::Block { available_bytes, .. } => {
                // Credited: 2 GiB live + 4 GiB resident = 6 GiB.
                assert_eq!(available_bytes, 6 * BYTES_PER_GIB);
            }
            MemoryGate::Proceed => panic!("expected Block"),
        }
    }

    #[test]
    fn build_model_fit_estimate_assembles_fields() {
        let estimate = build_model_fit_estimate(4 * BYTES_PER_GIB, 24 * BYTES_PER_GIB);
        assert_eq!(estimate.required_bytes, estimate_required_bytes(4 * BYTES_PER_GIB));
        assert_eq!(estimate.available_bytes, 24 * BYTES_PER_GIB);
        assert_eq!(estimate.verdict, MemoryFit::Comfortable);
    }
}
