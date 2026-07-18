//! Live-memory admission control for loading a model into the built-in engine.
//!
//! Issue #296: on a memory-constrained Mac, Thuki auto-loaded a large model
//! with no free-memory check, contributing to a whole-machine freeze. This
//! module adds a preflight that estimates a model's resident footprint, reads
//! how much system memory is actually available right now, and produces a
//! fit verdict the load paths gate on.
//!
//! The gate is deliberately *forgiving*: the footprint estimate is inherently
//! approximate (weights-on-disk plus optional mmproj size plus a fixed
//! overhead, ignoring the context-scaled KV cache, mirroring the
//! Library/Discover fit hint), and real-world estimators are documented to be
//! off by up to ~2x. So it only ever hard-blocks a load whose estimate lands
//! clearly above a ceiling of available memory, and a user-facing `force`
//! always bypasses it.
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
    MODEL_FIT_CEILING_FRACTION, MODEL_FIT_COMFORT_FRACTION, MODEL_FIT_HARD_BLOCK_FRACTION,
    RUNTIME_OVERHEAD_GB,
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
/// `weights_bytes` on disk and optional vision projector `mmproj_bytes`:
/// weights + mmproj + the fixed [`RUNTIME_OVERHEAD_GB`] (KV cache at the
/// default context plus runtime buffers). This is the byte-domain twin of
/// `super::estimate_runtime_gb_from_bytes`, kept on the same
/// weights-plus-overhead convention so the gate and the fit hint agree.
/// Pass `0` for text-only models. Saturating so a corrupt size cannot overflow.
pub fn estimate_required_bytes(weights_bytes: u64, mmproj_bytes: u64) -> u64 {
    // f64 -> u64 cast saturates at u64::MAX and floors negatives to 0; the
    // operand is a small positive constant so neither edge is reachable here.
    let overhead = (RUNTIME_OVERHEAD_GB * BYTES_PER_GIB as f64) as u64;
    weights_bytes
        .saturating_add(mmproj_bytes)
        .saturating_add(overhead)
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

/// True when `required_bytes` lands in the freeze band: at or above
/// [`MODEL_FIT_HARD_BLOCK_FRACTION`] times `available_bytes` (the estimate needs
/// at least that multiple of the memory free right now). A load in this band can
/// wire non-pageable Metal memory the machine does not have and hard-freeze
/// macOS, so the memory warning here can never be suppressed by a per-model
/// remember (only a per-turn `force` still bypasses it, upstream). The threshold
/// lives entirely in the constant so this predicate can never drift from its
/// value.
///
/// `available_bytes == 0` is a failed/unknown reader, not a real over-commit,
/// so it is never treated as the freeze band; the gate fails open on a 0 read
/// elsewhere and this keeps that decision coherent.
pub fn is_freeze_band(required_bytes: u64, available_bytes: u64) -> bool {
    if available_bytes == 0 {
        return false;
    }
    required_bytes as f64 >= MODEL_FIT_HARD_BLOCK_FRACTION * available_bytes as f64
}

/// True when `sha` (a model's weights SHA-256) is present in `dismissed_shas`,
/// the caller's snapshot of `behavior.dismissed_memory_fit_models`. The single
/// membership predicate the admission gate consults to decide whether a mild
/// over-limit load was already remembered by the user.
pub fn is_model_remembered(sha: &str, dismissed_shas: &[String]) -> bool {
    dismissed_shas.iter().any(|entry| entry == sha)
}

/// Applies a per-model "remember this model" override to a gate outcome.
///
/// The remember only rescues the MILD over-limit band: a [`MemoryGate::Block`]
/// whose figures are below the freeze band (see [`is_freeze_band`]) is
/// downgraded to [`MemoryGate::Proceed`] when the model is `dismissed`. A block
/// in the freeze band is returned unchanged even when `dismissed`, because free
/// RAM is dynamic and a gross over-commit must always re-warn (the reliability
/// floor). [`MemoryGate::Proceed`] passes through untouched: a forced load and
/// a comfortable/tight fit both already resolved to Proceed upstream, so there
/// is nothing to override. Pure and unit-tested; this is the single place the
/// dismissed flag changes a load decision.
pub fn apply_dismissed_override(gate: MemoryGate, dismissed: bool) -> MemoryGate {
    match gate {
        MemoryGate::Block {
            required_bytes,
            available_bytes,
        } if dismissed && !is_freeze_band(required_bytes, available_bytes) => MemoryGate::Proceed,
        other => other,
    }
}

/// Total on-disk primary weights bytes for an installed model: `size_bytes` for
/// a single-file model, or the sum of every shard for a split model (whose
/// `size_bytes` records only the first shard). Does not include the mmproj;
/// fold projector size via [`estimate_required_bytes`] or
/// [`model_load_bytes`]. Saturating on the sum.
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

/// Primary weights plus optional projector size for load-footprint estimates.
///
/// `mmproj_bytes` comes from the registry, a store blob length, or `0` when
/// unknown / text-only. Saturating so corrupt sizes cannot wrap.
pub fn model_load_bytes(model: &InstalledModel, mmproj_bytes: u64) -> u64 {
    model_weights_bytes(model).saturating_add(mmproj_bytes)
}

/// Projector size to fold into load estimates for `model`.
///
/// Returns `0` when the row has no mmproj. When it has one, prefers
/// `size_hint` (blob metadata or registry). Missing hint yields `0` rather
/// than inventing a size (conservative under-estimate, never a false block
/// from phantom projector bytes).
pub fn resolve_mmproj_bytes(model: &InstalledModel, size_hint: Option<u64>) -> u64 {
    if model.mmproj_sha256.is_none() {
        return 0;
    }
    size_hint.unwrap_or(0)
}

/// Projector bytes to fold into `model`'s load estimate: the store blob length
/// when the mmproj blob is present, else the curated registry size, else `0`.
///
/// The single source of the blob-then-registry hint so the pre-load memory gate
/// ([`crate::commands::preflight_memory_gate`]) and the fit estimate
/// ([`estimate_model_fit`]) can never size the same projector differently.
///
/// Coverage-off: a thin filesystem + registry read. The size composition is the
/// unit-tested [`resolve_mmproj_bytes`]; the blob-then-registry preference is
/// `Option::or_else`.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn mmproj_bytes_for_model(
    store: &crate::models::storage::ModelStore,
    model: &InstalledModel,
) -> u64 {
    let hint = model.mmproj_sha256.as_deref().and_then(|sha| {
        std::fs::metadata(store.blob_path(sha))
            .ok()
            .map(|m| m.len())
            .or_else(|| {
                super::registry::by_repo_file(&model.repo, &model.file_name).map(|s| s.mmproj_bytes)
            })
    });
    resolve_mmproj_bytes(model, hint)
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

/// Available memory adjusted for a resident model that would be evicted before
/// loading a *different* target: credits the resident model's weight bytes back
/// (the engine frees the current model before allocating the next), so the
/// admission gate and any fit estimate shown to the user agree on the same
/// effective figure.
///
/// Returns `available_bytes` unchanged when nothing is resident, or when the
/// resident model *is* the target itself (no eviction happens, so no credit).
/// Saturating so an adversarial installed-row size can never wrap the sum into
/// a small (falsely "insufficient") number.
///
/// Callers that fail open on `available_bytes == 0` (an unreadable memory
/// reader) must apply that check *before* calling this: crediting a resident
/// model onto a 0 read judges the load against the credit alone, which is
/// incoherent with "we don't know available memory". This helper does no such
/// guarding, so both the gate and the fit estimate keep that decision explicit
/// at their own call sites.
pub fn effective_available_bytes(
    available_bytes: u64,
    resident_path: Option<&Path>,
    target_path: &Path,
    installed: &[(u64, PathBuf)],
) -> u64 {
    let credit = match resident_path {
        // Already serving the exact model: reused in place, nothing evicted.
        Some(path) if path == target_path => 0,
        // A different model is resident and freed before the new load.
        Some(path) => resident_weights_bytes(path, installed),
        None => 0,
    };
    available_bytes.saturating_add(credit)
}

/// True when the engine is already mid-load for `target_path`: state
/// `"starting"` with a model_path that matches exactly. Such a load was
/// already admitted by an earlier gate check (e.g. auto-prime at boot, when
/// more memory was free) and is still streaming in; re-running the gate here
/// would judge that same admitted load against memory the load itself has
/// already spent, spuriously blocking a load already underway and on track
/// to finish (issue #296 race). Deliberately exact-match only: a DIFFERENT
/// model that happens to be `"starting"` is not treated as any kind of
/// resident/creditable state here, since an incomplete load's actual freed
/// footprint on eviction is not well-defined the way a fully-loaded model's
/// is: that stays exactly as conservative as it is today.
pub fn is_target_already_loading(state: &str, status_model_path: &str, target_path: &Path) -> bool {
    state == "starting"
        && !status_model_path.is_empty()
        && Path::new(status_model_path) == target_path
}

/// The admission decision for loading the model at `target_path` (weights
/// `target_weights_bytes`) given the memory available right now.
///
/// - `forced` (the user's "load anyway") short-circuits to [`MemoryGate::Proceed`].
/// - `available_bytes == 0` (the reader failed) fails open to
///   [`MemoryGate::Proceed`] before any credit arithmetic, so a bad read can
///   never brick a load regardless of what is resident.
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
    // Fail open on a failed reader (0) before any credit arithmetic. `assess_fit`
    // treats 0 as "unknown, never block", but crediting a resident model adds a
    // positive figure that would push a raw-0 read off that zero-shortcut and
    // judge the load against the credit alone, flipping the same "we don't know
    // available memory" condition from Proceed to Block depending on incidental
    // residency. Short-circuiting here keeps the fail-open contract coherent
    // regardless of residency; the launch circuit breaker (`startup_guard`) is
    // the backstop if a bad-read Proceed ever contributes to a freeze.
    if available_bytes == 0 {
        return MemoryGate::Proceed;
    }
    // Already serving the exact model: reused in place, nothing to admit. Kept
    // as an early return (skipping all arithmetic) so a model that already fills
    // memory can never be blocked from continuing to serve.
    if matches!(resident_path, Some(path) if path == target_path) {
        return MemoryGate::Proceed;
    }
    // Callers fold mmproj into `target_weights_bytes` (or pass primary-only when
    // projector size is unknown); second arg stays 0 so the sum is not doubled.
    let required_bytes = estimate_required_bytes(target_weights_bytes, 0);
    // A different resident model (or none) is credited back by the shared
    // helper, so the gate and the frontend fit estimate use identical math.
    let effective_available =
        effective_available_bytes(available_bytes, resident_path, target_path, installed);
    match assess_fit(required_bytes, effective_available) {
        MemoryFit::Insufficient => MemoryGate::Block {
            required_bytes,
            available_bytes: effective_available,
        },
        MemoryFit::Comfortable | MemoryFit::Tight => MemoryGate::Proceed,
    }
}

/// The single authoritative "would loading this target be blocked?" decision,
/// shared by [`preflight_memory_gate`] (the real admission gate) and
/// [`build_model_fit_estimate`] (the frontend's fit affordance) so exactly one
/// copy of the block logic exists. A duplicated gate that drifted and froze a
/// Mac is the reason issue #296 exists, so the estimate path and the gate must
/// never compute this decision separately.
///
/// Folds in the already-loading (`"starting"` state naming this exact target)
/// proceed short-circuit that used to live only in `preflight_memory_gate`,
/// then delegates the forced / failed-reader / resident-exact / memory-fit
/// judgement to [`evaluate_load_gate`]. `available_bytes` is the raw live
/// reading; the resident-eviction credit is applied inside `evaluate_load_gate`.
///
/// [`preflight_memory_gate`]: crate::commands::preflight_memory_gate
#[allow(clippy::too_many_arguments)]
pub fn decide_load_gate(
    engine_state: &str,
    status_model_path: &str,
    target_weights_bytes: u64,
    available_bytes: u64,
    resident_path: Option<&Path>,
    target_path: &Path,
    installed: &[(u64, PathBuf)],
    forced: bool,
) -> MemoryGate {
    // why: a load already mid-flight for this exact target was admitted by an
    // earlier gate check and is still streaming in; re-judging it against the
    // memory that same load has already spent would spuriously block a load on
    // track to finish (issue #296 race).
    if is_target_already_loading(engine_state, status_model_path, target_path) {
        return MemoryGate::Proceed;
    }
    evaluate_load_gate(
        target_weights_bytes,
        available_bytes,
        resident_path,
        target_path,
        installed,
        forced,
    )
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
    // `libc` 0.2.186 exports `mach_host_self`/`host_statistics64`/`mach_task_self`
    // but not `mach_port_deallocate`, so declare it directly. It lives in
    // libSystem (always linked on macOS), and releases the host send right that
    // `mach_host_self` hands the caller.
    unsafe extern "C" {
        fn mach_port_deallocate(
            task: libc::mach_port_t,
            name: libc::mach_port_t,
        ) -> libc::kern_return_t;
    }

    // SAFETY: `page_size` is guarded before the host port is acquired, so a
    // `sysconf` failure never leaks a send right. `mach_host_self` returns a
    // host port the caller owns; `stats` is a valid, correctly-sized stack
    // buffer and `count` is initialized to the element count
    // `host_statistics64` expects for `HOST_VM_INFO64`, read/written in place.
    // The host port is released via `mach_port_deallocate` on every path once
    // the read is done (the send right is dead after `host_statistics64`
    // returns, whatever its status); `mach_task_self` is not caller-owned and is
    // never deallocated. Every non-success return is mapped to 0, so no
    // uninitialized data is ever consumed.
    unsafe {
        let page_size = libc::sysconf(libc::_SC_PAGESIZE);
        if page_size <= 0 {
            return 0;
        }
        let host = libc::mach_host_self();
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
        // Release the host send right on every path out. A deallocate failure
        // cannot be recovered from and must not mask the memory read, but it
        // signals a real port-management problem, so log it rather than swallow
        // it (mirrors `startup_guard`'s forgiving-boundary eprintln convention).
        let dr = mach_port_deallocate(libc::mach_task_self(), host);
        if dr != libc::KERN_SUCCESS {
            eprintln!("thuki: [memory] mach_port_deallocate(host) failed: {dr}");
        }
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
    /// Memory judged available for the load, in bytes (already crediting a
    /// resident model that would be evicted first, matching the gate).
    pub available_bytes: u64,
    /// The fit verdict for the two figures above.
    pub verdict: MemoryFit,
    /// Whether the real admission gate would block this load right now. The
    /// authoritative signal the model-switch flow branches on (issue #296): the
    /// display `verdict` alone can read `Insufficient` for a target the gate
    /// would still admit (a resident-exact or already-loading model), so a
    /// control decision must never be derived from `verdict`.
    pub would_block: bool,
    /// Whether a per-model "remember" could suppress this warning: true unless
    /// the load is in the freeze band ([`is_freeze_band`]). The single source of
    /// truth for the frontend's "Always allow this model" action, so the
    /// freeze-floor fraction is never duplicated as a magic number on the
    /// frontend. A freeze-band load is never remember-able, so that action is
    /// hidden for it.
    pub can_remember: bool,
}

/// Pure core of [`estimate_model_fit`]: assembles the full [`ModelFitEstimate`]
/// the frontend renders (footprint, credited available, display verdict) AND
/// the authoritative `would_block` decision the model-switch flow branches on
/// (issue #296). Split from the coverage-off command so every field, including
/// the resident-credit branch and the gate decision, is unit-tested without a
/// Tauri runtime.
///
/// `raw_available_bytes` is the live reading straight from the memory FFI; the
/// resident-eviction credit is applied here (via the shared
/// [`effective_available_bytes`]) so the displayed `available_bytes`, the
/// display `verdict`, and `would_block` all agree with the real admission gate
/// on identical inputs. `would_block` comes from [`decide_load_gate`], the
/// single source of the block decision shared with `preflight_memory_gate`, so
/// no second copy of that logic can drift.
#[allow(clippy::too_many_arguments)]
pub fn build_model_fit_estimate(
    engine_state: &str,
    status_model_path: &str,
    weights_bytes: u64,
    raw_available_bytes: u64,
    resident_path: Option<&Path>,
    target_path: &Path,
    installed: &[(u64, PathBuf)],
) -> ModelFitEstimate {
    // Callers fold mmproj into `weights_bytes` when known; second arg stays 0.
    let required_bytes = estimate_required_bytes(weights_bytes, 0);
    // Fail open on a failed reader (0) before crediting, mirroring the gate:
    // crediting a resident model onto a 0 read would judge the estimate against
    // the credit alone, diverging from "we don't know available memory".
    let effective_available = if raw_available_bytes == 0 {
        0
    } else {
        effective_available_bytes(raw_available_bytes, resident_path, target_path, installed)
    };
    // why: single source of the block decision. Never re-derive "would block"
    // from the display verdict below: the gate's resident-exact and
    // already-loading proceed short-circuits mean a target can read
    // `Insufficient` for display yet still be admitted (issue #296 drift).
    let would_block = matches!(
        decide_load_gate(
            engine_state,
            status_model_path,
            weights_bytes,
            raw_available_bytes,
            resident_path,
            target_path,
            installed,
            false,
        ),
        MemoryGate::Block { .. }
    );
    ModelFitEstimate {
        required_bytes,
        available_bytes: effective_available,
        verdict: assess_fit(required_bytes, effective_available),
        would_block,
        // Computed against the SAME credited available the gate blocks on, so the
        // remember action is hidden for exactly the loads
        // `apply_dismissed_override` refuses to suppress.
        can_remember: !is_freeze_band(required_bytes, effective_available),
    }
}

/// Estimates whether a model fits in the memory available right now, for the
/// frontend's "may not fit, load anyway?" affordance. `model_id` names the
/// installed model, or the active provider's model when omitted.
///
/// Thin Tauri wrapper (coverage-off): resolves the model id, reads its weights
/// size from the manifest, gathers the same resident/installed/live-memory
/// inputs [`preflight_memory_gate`] uses, and hands them to the unit-tested
/// [`build_model_fit_estimate`]. That helper applies the resident-eviction
/// credit (issue #296: the gate credited but the display did not, so switching
/// models showed a pessimistic "available") and, via [`decide_load_gate`],
/// produces `would_block` from the SAME block decision the admission gate runs,
/// so the frontend's model-switch flow never re-derives it and drifts. The
/// inline resident-detection branch matches `preflight_memory_gate` (also
/// coverage-off) exactly.
///
/// [`preflight_memory_gate`]: crate::commands::preflight_memory_gate
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn estimate_model_fit(
    model_id: Option<String>,
    db: tauri::State<'_, crate::history::Database>,
    config: tauri::State<'_, parking_lot::RwLock<crate::config::AppConfig>>,
    engine: tauri::State<'_, crate::engine::runner::EngineHandle>,
    store: tauri::State<'_, crate::models::storage::ModelStore>,
) -> Result<ModelFitEstimate, String> {
    let model_id =
        model_id.unwrap_or_else(|| config.read().inference.active_provider_model().to_string());
    if model_id.is_empty() {
        return Err("No model selected.".to_string());
    }
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    let row = super::manifest::get(&conn, &model_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "The selected model is not installed.".to_string())?;
    // Fold mmproj blob length / registry size so vision fit matches the gate.
    let weights_bytes = model_load_bytes(&row, mmproj_bytes_for_model(store.inner(), &row));
    // The target's weights blob path; for a single-file model this equals the
    // resident `model_path` the engine reports, giving exact credit parity with
    // the gate. Split models load through a shim path that never matches a blob
    // path here, which only credits nothing (never a false pass).
    let target_path = store.blob_path(&row.sha256);
    // Map every installed row to (weights_bytes, weights blob path) so a resident
    // model can be matched by path and its footprint credited back, mirroring
    // `preflight_memory_gate`.
    let installed: Vec<(u64, PathBuf)> = super::manifest::list(&conn)
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            let mm = mmproj_bytes_for_model(store.inner(), &r);
            (model_load_bytes(&r, mm), store.blob_path(&r.sha256))
        })
        .collect();
    // A live "loaded" status names the resident model's path; anything else
    // means nothing is resident to credit. The `"starting"` (already-loading)
    // bypass is applied inside `build_model_fit_estimate` -> `decide_load_gate`
    // from this same status, mirroring `preflight_memory_gate`.
    let status = engine.current_status();
    let resident = (status.state == "loaded" && !status.model_path.is_empty())
        .then(|| PathBuf::from(&status.model_path));
    Ok(build_model_fit_estimate(
        &status.state,
        &status.model_path,
        weights_bytes,
        available_memory_bytes(),
        resident.as_deref(),
        &target_path,
        &installed,
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
    fn available_memory_bytes_reads_a_plausible_live_value() {
        // Exercises the real mach FFI end to end (coverage-off, so the pure
        // suite never touches it): the syscall must return a live, plausible
        // figure, not the 0 that a wrong field/count wiring would yield and
        // that `assess_fit` would silently treat as "always fits". A running
        // machine always has some reclaimable pages, and available memory can
        // never exceed total physical RAM. Guards against a libc/struct
        // regression that would turn the whole gate into a no-op.
        let available = available_memory_bytes();
        assert!(available > 0, "expected some available memory, got 0");
        let total = super::super::system_ram_bytes();
        assert!(
            available <= total,
            "available {available} should not exceed total RAM {total}",
        );
    }

    #[test]
    fn available_from_vm_stats_saturates_instead_of_overflowing() {
        // A pathological snapshot must never wrap into a small number.
        assert_eq!(
            available_from_vm_stats(u64::MAX, u64::MAX, 0, 0, 2),
            u64::MAX
        );
        assert_eq!(
            available_from_vm_stats(u64::MAX, 0, 0, 0, u64::MAX),
            u64::MAX
        );
    }

    #[test]
    fn estimate_required_bytes_adds_fixed_overhead() {
        let overhead = (RUNTIME_OVERHEAD_GB * BYTES_PER_GIB as f64) as u64;
        assert_eq!(estimate_required_bytes(0, 0), overhead);
        assert_eq!(
            estimate_required_bytes(5 * BYTES_PER_GIB, 0),
            5 * BYTES_PER_GIB + overhead
        );
    }

    #[test]
    fn estimate_required_bytes_includes_mmproj() {
        let overhead = (RUNTIME_OVERHEAD_GB * BYTES_PER_GIB as f64) as u64;
        assert_eq!(
            estimate_required_bytes(5 * BYTES_PER_GIB, BYTES_PER_GIB),
            6 * BYTES_PER_GIB + overhead
        );
    }

    #[test]
    fn estimate_required_bytes_saturates() {
        assert_eq!(estimate_required_bytes(u64::MAX, 0), u64::MAX);
        assert_eq!(estimate_required_bytes(u64::MAX, 1), u64::MAX);
    }

    #[test]
    fn model_load_bytes_adds_mmproj() {
        let model = row(1000, vec![]);
        assert_eq!(model_load_bytes(&model, 0), 1000);
        assert_eq!(model_load_bytes(&model, 250), 1250);
        let split = row(100, vec![part("a", 100), part("b", 200)]);
        assert_eq!(model_load_bytes(&split, 50), 350);
    }

    #[test]
    fn resolve_mmproj_bytes_respects_presence_and_hint() {
        let text = row(1000, vec![]);
        assert_eq!(resolve_mmproj_bytes(&text, Some(99)), 0);
        let mut vision = row(1000, vec![]);
        vision.mmproj_sha256 = Some("sha_mm".into());
        vision.mmproj_file = Some("mmproj.gguf".into());
        assert_eq!(resolve_mmproj_bytes(&vision, None), 0);
        assert_eq!(resolve_mmproj_bytes(&vision, Some(500)), 500);
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
        assert_eq!(
            assess_fit(5 * BYTES_PER_GIB, available),
            MemoryFit::Comfortable
        );
        assert_eq!(
            assess_fit(
                (MODEL_FIT_COMFORT_FRACTION * available as f64) as u64,
                available
            ),
            MemoryFit::Comfortable
        );
        // Tight: above comfort, at/under the ceiling (0.80 -> 8 GiB).
        assert_eq!(assess_fit(7 * BYTES_PER_GIB, available), MemoryFit::Tight);
        assert_eq!(
            assess_fit(
                (MODEL_FIT_CEILING_FRACTION * available as f64) as u64,
                available
            ),
            MemoryFit::Tight
        );
        // Insufficient: clearly above the ceiling.
        assert_eq!(
            assess_fit(9 * BYTES_PER_GIB, available),
            MemoryFit::Insufficient
        );
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
        assert_eq!(
            resident_weights_bytes(Path::new("/blobs/b"), &installed),
            20
        );
    }

    #[test]
    fn resident_weights_bytes_no_match_is_zero() {
        let installed = vec![(10u64, PathBuf::from("/blobs/a"))];
        // An unknown path (e.g. a split-shim symlink) credits nothing.
        assert_eq!(
            resident_weights_bytes(Path::new("/blobs/shim"), &installed),
            0
        );
    }

    #[test]
    fn effective_available_same_model_resident_adds_no_credit() {
        // The resident model IS the target: no eviction happens, so the raw
        // available figure is returned unchanged.
        let target = PathBuf::from("/blobs/target");
        let installed = vec![(9 * BYTES_PER_GIB, target.clone())];
        assert_eq!(
            effective_available_bytes(BYTES_PER_GIB, Some(&target), &target, &installed),
            BYTES_PER_GIB
        );
    }

    #[test]
    fn effective_available_different_resident_is_credited() {
        // A different resident model matched by blob path credits its weights
        // back, since the engine evicts it before the new load allocates.
        let a = PathBuf::from("/blobs/a");
        let b = PathBuf::from("/blobs/b");
        let installed = vec![(14 * BYTES_PER_GIB, a.clone())];
        assert_eq!(
            effective_available_bytes(BYTES_PER_GIB, Some(&a), &b, &installed),
            BYTES_PER_GIB + 14 * BYTES_PER_GIB
        );
    }

    #[test]
    fn effective_available_no_resident_is_unchanged() {
        // Nothing resident: nothing to credit.
        let target = PathBuf::from("/blobs/target");
        let installed = vec![(14 * BYTES_PER_GIB, PathBuf::from("/blobs/a"))];
        assert_eq!(
            effective_available_bytes(3 * BYTES_PER_GIB, None, &target, &installed),
            3 * BYTES_PER_GIB
        );
    }

    #[test]
    fn effective_available_saturates_on_adversarial_credit() {
        // A pathological installed-row size must never wrap the sum small.
        let a = PathBuf::from("/blobs/a");
        let b = PathBuf::from("/blobs/b");
        let installed = vec![(u64::MAX, a.clone())];
        assert_eq!(
            effective_available_bytes(u64::MAX, Some(&a), &b, &installed),
            u64::MAX
        );
    }

    #[test]
    fn effective_available_unknown_resident_path_credits_nothing() {
        // A resident path absent from `installed` (e.g. a split-shim symlink)
        // credits nothing, matching `resident_weights_bytes`.
        let shim = PathBuf::from("/blobs/shim");
        let target = PathBuf::from("/blobs/target");
        let installed = vec![(14 * BYTES_PER_GIB, PathBuf::from("/blobs/a"))];
        assert_eq!(
            effective_available_bytes(2 * BYTES_PER_GIB, Some(&shim), &target, &installed),
            2 * BYTES_PER_GIB
        );
    }

    #[test]
    fn estimate_reflects_resident_credit_on_switch() {
        // Reproduces the issue #296 display gap with the user's live numbers:
        // gemma (~11.28 GiB weights) is resident and had just answered; the user
        // switches the active model to gpt-oss 20B (12_109_566_560 B weights) and
        // the memory reader sees only ~1.7 GB free because gemma still fills VRAM.
        //
        // gemma is evicted before gpt-oss loads, so the fit estimate the card
        // shows must credit gemma's footprint back. This asserts the same
        // `build_model_fit_estimate` core the command feeds, once WITHOUT credit
        // (the pre-fix display) and once WITH the shared helper's credit.
        let gpt_oss_weights: u64 = 12_109_566_560;
        let gemma_weights: u64 = 12_112_665_600; // ~11.28 GiB
        let raw_available: u64 = 1_825_361_100; // ~1.7 GB, as displayed
        let gemma_path = PathBuf::from("/blobs/gemma");
        let gpt_oss_path = PathBuf::from("/blobs/gpt-oss");
        let installed = vec![
            (gemma_weights, gemma_path.clone()),
            (gpt_oss_weights, gpt_oss_path.clone()),
        ];

        // Pre-fix: no credit -> the card shows the raw ~1.7 GB and blocks.
        let without_credit = build_model_fit_estimate(
            "loaded",
            "",
            gpt_oss_weights,
            raw_available,
            None,
            &gpt_oss_path,
            &installed,
        );
        assert_eq!(without_credit.available_bytes, raw_available);
        assert_eq!(without_credit.verdict, MemoryFit::Insufficient);

        // Post-fix: gemma resident and credited back before gpt-oss loads. The
        // helper now applies the credit internally from the resident path.
        let with_credit = build_model_fit_estimate(
            "loaded",
            &gemma_path.to_string_lossy(),
            gpt_oss_weights,
            raw_available,
            Some(&gemma_path),
            &gpt_oss_path,
            &installed,
        );
        // The displayed "available" now reflects the eviction, up by exactly
        // gemma's weights, matching what the admission gate already computed.
        assert_eq!(with_credit.available_bytes, raw_available + gemma_weights);
        assert_eq!(
            with_credit.available_bytes - without_credit.available_bytes,
            gemma_weights
        );
    }

    #[test]
    fn is_target_already_loading_starting_matching_path_is_true() {
        // The engine is mid-load for this exact target: an already-admitted
        // in-flight load must not be re-judged (issue #296 race).
        assert!(is_target_already_loading(
            "starting",
            "/blobs/target",
            Path::new("/blobs/target")
        ));
    }

    #[test]
    fn is_target_already_loading_starting_different_path_is_false() {
        // A different model mid-load is not treated as resident/creditable here.
        assert!(!is_target_already_loading(
            "starting",
            "/blobs/other",
            Path::new("/blobs/target")
        ));
    }

    #[test]
    fn is_target_already_loading_loaded_matching_path_is_false() {
        // A fully-loaded matching model is the existing resident-credit path's
        // job (`evaluate_load_gate`), not this in-flight bypass.
        assert!(!is_target_already_loading(
            "loaded",
            "/blobs/target",
            Path::new("/blobs/target")
        ));
    }

    #[test]
    fn is_target_already_loading_non_starting_states_are_false() {
        // Nothing loading: neither a stopped nor a failed engine bypasses.
        assert!(!is_target_already_loading(
            "stopped",
            "/blobs/target",
            Path::new("/blobs/target")
        ));
        assert!(!is_target_already_loading(
            "failed",
            "/blobs/target",
            Path::new("/blobs/target")
        ));
    }

    #[test]
    fn is_target_already_loading_empty_path_is_false() {
        // A "starting" status with no model_path yet cannot match any target.
        assert!(!is_target_already_loading(
            "starting",
            "",
            Path::new("/blobs/target")
        ));
    }

    #[test]
    fn gate_forced_always_proceeds() {
        // Force bypasses even a clearly-oversized model against no memory.
        let gate = evaluate_load_gate(u64::MAX, 1, None, Path::new("/blobs/target"), &[], true);
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
        assert_eq!(
            gate,
            MemoryGate::Block {
                required_bytes: estimate_required_bytes(20 * BYTES_PER_GIB, 0),
                available_bytes: 10 * BYTES_PER_GIB,
            }
        );
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
        let gate = evaluate_load_gate(20 * BYTES_PER_GIB, 0, Some(&target), &target, &[], false);
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
        assert_eq!(
            evaluate_load_gate(
                8 * BYTES_PER_GIB,
                BYTES_PER_GIB,
                None,
                &b,
                &installed,
                false
            ),
            MemoryGate::Block {
                required_bytes: estimate_required_bytes(8 * BYTES_PER_GIB, 0),
                available_bytes: BYTES_PER_GIB,
            }
        );
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
        // Credited: 2 GiB live + 4 GiB resident = 6 GiB, still far short.
        assert_eq!(
            gate,
            MemoryGate::Block {
                required_bytes: estimate_required_bytes(30 * BYTES_PER_GIB, 0),
                available_bytes: 6 * BYTES_PER_GIB,
            }
        );
    }

    #[test]
    fn gate_fails_open_on_zero_available_even_with_resident_credit() {
        // A raw available read of 0 means the FFI read failed; the gate must
        // fail open regardless of residency. With a different model resident,
        // the credit would otherwise push the judged figure off `assess_fit`'s
        // zero-shortcut: pre-fix, 0 live + 14 GiB credit judged a ~22 GiB model
        // Insufficient -> Block. The front short-circuit keeps it coherent.
        let a = PathBuf::from("/blobs/a");
        let b = PathBuf::from("/blobs/b");
        let installed = vec![(14 * BYTES_PER_GIB, a.clone())];
        let gate = evaluate_load_gate(20 * BYTES_PER_GIB, 0, Some(&a), &b, &installed, false);
        assert_eq!(gate, MemoryGate::Proceed);
    }

    #[test]
    fn build_model_fit_estimate_assembles_fields() {
        // Nothing resident, comfortable fit: fields assemble and the gate would
        // not block.
        let target = PathBuf::from("/blobs/target");
        let estimate = build_model_fit_estimate(
            "stopped",
            "",
            4 * BYTES_PER_GIB,
            24 * BYTES_PER_GIB,
            None,
            &target,
            &[],
        );
        assert_eq!(
            estimate.required_bytes,
            estimate_required_bytes(4 * BYTES_PER_GIB, 0)
        );
        assert_eq!(estimate.available_bytes, 24 * BYTES_PER_GIB);
        assert_eq!(estimate.verdict, MemoryFit::Comfortable);
        assert!(!estimate.would_block);
        // Well under the freeze floor, so a remember could suppress it.
        assert!(estimate.can_remember);
    }

    #[test]
    fn decide_load_gate_bypasses_already_loading_target() {
        // A "starting" status naming this exact target: an in-flight load
        // already admitted must not be re-judged, even against no memory and an
        // oversized model (issue #296 race). This is the short-circuit
        // `evaluate_load_gate` alone does NOT have.
        let target = PathBuf::from("/blobs/target");
        let gate = decide_load_gate(
            "starting",
            "/blobs/target",
            u64::MAX,
            1,
            None,
            &target,
            &[],
            false,
        );
        assert_eq!(gate, MemoryGate::Proceed);
    }

    #[test]
    fn decide_load_gate_matches_evaluate_when_not_already_loading() {
        // When the already-loading bypass does not fire, the decision must be
        // byte-for-byte the same as `evaluate_load_gate` on the same inputs, so
        // the two callers can never diverge (issue #296).
        let target = PathBuf::from("/blobs/target");
        let inputs = (20 * BYTES_PER_GIB, 10 * BYTES_PER_GIB);
        // "loaded" state (not "starting") -> no bypass -> pure delegation.
        let decided = decide_load_gate(
            "loaded",
            "/blobs/other",
            inputs.0,
            inputs.1,
            None,
            &target,
            &[],
            false,
        );
        let evaluated = evaluate_load_gate(inputs.0, inputs.1, None, &target, &[], false);
        assert_eq!(decided, evaluated);
        assert_eq!(
            decided,
            MemoryGate::Block {
                required_bytes: estimate_required_bytes(20 * BYTES_PER_GIB, 0),
                available_bytes: 10 * BYTES_PER_GIB,
            }
        );
    }

    #[test]
    fn decide_load_gate_forced_admits_where_unforced_blocks() {
        // The consented "Load anyway" force (issue #296) must be admitted
        // through the SAME gate decision the ambient warm-up uses: the exact
        // oversized inputs that block unforced (`forced=false` -> Block) proceed
        // when forced (`forced=true` -> Proceed), with no separate bypass path.
        let target = PathBuf::from("/blobs/target");
        let unforced = decide_load_gate(
            "loaded",
            "/blobs/other",
            20 * BYTES_PER_GIB,
            10 * BYTES_PER_GIB,
            None,
            &target,
            &[],
            false,
        );
        assert_eq!(
            unforced,
            MemoryGate::Block {
                required_bytes: estimate_required_bytes(20 * BYTES_PER_GIB, 0),
                available_bytes: 10 * BYTES_PER_GIB,
            }
        );
        let forced = decide_load_gate(
            "loaded",
            "/blobs/other",
            20 * BYTES_PER_GIB,
            10 * BYTES_PER_GIB,
            None,
            &target,
            &[],
            true,
        );
        assert_eq!(forced, MemoryGate::Proceed);
    }

    #[test]
    fn estimate_would_block_true_for_oversized_cold_load() {
        // 20 GiB model, 10 GiB available, nothing resident: the gate blocks and
        // `would_block` reports it.
        let target = PathBuf::from("/blobs/target");
        let estimate = build_model_fit_estimate(
            "loaded",
            "/blobs/other",
            20 * BYTES_PER_GIB,
            10 * BYTES_PER_GIB,
            None,
            &target,
            &[],
        );
        assert_eq!(estimate.verdict, MemoryFit::Insufficient);
        assert!(estimate.would_block);
        // 22 GiB needed / 10 GiB free = ratio 2.20: over the ceiling (so it
        // blocks) but under the 3x freeze floor, so it stays remember-able.
        // `would_block` and `can_remember` are independent by design.
        assert!(estimate.can_remember);

        // Push the same load into the freeze band (42 GiB needed / 10 GiB free
        // = ratio 4.20): still blocked, and no longer remember-able, so the
        // frontend hides the "Always allow this model" action.
        let freeze = build_model_fit_estimate(
            "loaded",
            "/blobs/other",
            40 * BYTES_PER_GIB,
            10 * BYTES_PER_GIB,
            None,
            &target,
            &[],
        );
        assert!(freeze.would_block);
        assert!(!freeze.can_remember);
    }

    #[test]
    fn estimate_would_block_false_for_resident_exact_target() {
        // THE divergence issue #296 Option B fixes: the user re-picks the model
        // that is already resident and filling memory (available reads ~0). The
        // display `verdict` is `Insufficient` (its footprint is not in the free
        // reading), but the gate proceeds on a same-model reload, so
        // `would_block` MUST be false. Branching on `verdict` here would refuse
        // to reload a working model.
        let target = PathBuf::from("/blobs/target");
        let estimate = build_model_fit_estimate(
            "loaded",
            "/blobs/target",
            20 * BYTES_PER_GIB,
            0,
            Some(&target),
            &target,
            &[],
        );
        assert!(!estimate.would_block);
    }

    #[test]
    fn estimate_would_block_false_for_already_loading_target() {
        // The exact target is mid-load ("starting"): already admitted, so
        // `would_block` is false regardless of the (pessimistic) display verdict.
        let target = PathBuf::from("/blobs/target");
        let estimate = build_model_fit_estimate(
            "starting",
            "/blobs/target",
            20 * BYTES_PER_GIB,
            1,
            None,
            &target,
            &[],
        );
        assert!(!estimate.would_block);
    }

    #[test]
    fn estimate_would_block_false_for_comfortable_fit() {
        // Plenty of headroom, nothing resident: fits and the gate proceeds.
        let target = PathBuf::from("/blobs/target");
        let estimate = build_model_fit_estimate(
            "loaded",
            "/blobs/other",
            4 * BYTES_PER_GIB,
            24 * BYTES_PER_GIB,
            None,
            &target,
            &[],
        );
        assert_eq!(estimate.verdict, MemoryFit::Comfortable);
        assert!(!estimate.would_block);
    }

    #[test]
    fn estimate_would_block_agrees_with_gate_on_switch_to_oversized() {
        // Switch A(4 GiB, resident) -> B(30 GiB) on a tight machine: the gate
        // credits A back and still blocks. `would_block` must equal the gate's
        // own decision on the identical inputs.
        let a = PathBuf::from("/blobs/a");
        let b = PathBuf::from("/blobs/b");
        let installed = vec![(4 * BYTES_PER_GIB, a.clone())];
        let estimate = build_model_fit_estimate(
            "loaded",
            &a.to_string_lossy(),
            30 * BYTES_PER_GIB,
            2 * BYTES_PER_GIB,
            Some(&a),
            &b,
            &installed,
        );
        let gate = decide_load_gate(
            "loaded",
            &a.to_string_lossy(),
            30 * BYTES_PER_GIB,
            2 * BYTES_PER_GIB,
            Some(&a),
            &b,
            &installed,
            false,
        );
        assert_eq!(
            estimate.would_block,
            matches!(gate, MemoryGate::Block { .. })
        );
        assert!(estimate.would_block);
    }

    #[test]
    fn is_freeze_band_triggers_at_or_above_triple_available() {
        // Ratio exactly 3.00 (required == 3x available) is the freeze floor
        // (>=, so the boundary itself is freeze).
        assert!(is_freeze_band(30 * BYTES_PER_GIB, 10 * BYTES_PER_GIB));
        // Above 3.00.
        assert!(is_freeze_band(40 * BYTES_PER_GIB, 10 * BYTES_PER_GIB));
        // Just under the floor (ratio 2.90) is still rememberable.
        assert!(!is_freeze_band(29 * BYTES_PER_GIB, 10 * BYTES_PER_GIB));
        // Mid-band (ratio 2.00) and the user's "squeeze it out" case (1.10):
        // over the mild ceiling but below the freeze floor, so NOT freeze.
        assert!(!is_freeze_band(20 * BYTES_PER_GIB, 10 * BYTES_PER_GIB));
        assert!(!is_freeze_band(11 * BYTES_PER_GIB, 10 * BYTES_PER_GIB));
        // A failed/unknown reader (0 available) is never treated as freeze.
        assert!(!is_freeze_band(30 * BYTES_PER_GIB, 0));
    }

    #[test]
    fn is_model_remembered_matches_list_membership() {
        let sha_a = "a".repeat(64);
        let sha_b = "b".repeat(64);
        let list = vec![sha_a.clone()];
        assert!(is_model_remembered(&sha_a, &list));
        assert!(!is_model_remembered(&sha_b, &list));
        // Empty list: nothing is remembered.
        assert!(!is_model_remembered(&sha_a, &[]));
    }

    #[test]
    fn apply_dismissed_override_proceeds_pass_through() {
        // Proceed (forced load, or a comfortable/tight fit) is never touched,
        // regardless of the dismissed flag.
        assert_eq!(
            apply_dismissed_override(MemoryGate::Proceed, true),
            MemoryGate::Proceed
        );
        assert_eq!(
            apply_dismissed_override(MemoryGate::Proceed, false),
            MemoryGate::Proceed
        );
    }

    #[test]
    fn apply_dismissed_override_rescues_mild_band_only_when_dismissed() {
        // The user's "squeeze it out" case: over the mild ceiling at ratio 1.10
        // (11 needed / 10 free), below the 3x freeze floor. Dismissed
        // downgrades to Proceed; not-dismissed still blocks.
        let mild = MemoryGate::Block {
            required_bytes: 11 * BYTES_PER_GIB,
            available_bytes: 10 * BYTES_PER_GIB,
        };
        assert_eq!(apply_dismissed_override(mild, true), MemoryGate::Proceed);
        assert_eq!(apply_dismissed_override(mild, false), mild);

        // The newly-widened middle: ratio 2.00 is still under the 3x floor, so
        // it is rememberable too.
        let mid = MemoryGate::Block {
            required_bytes: 20 * BYTES_PER_GIB,
            available_bytes: 10 * BYTES_PER_GIB,
        };
        assert_eq!(apply_dismissed_override(mid, true), MemoryGate::Proceed);
        assert_eq!(apply_dismissed_override(mid, false), mid);
    }

    #[test]
    fn apply_dismissed_override_never_rescues_freeze_band() {
        // RELIABILITY FLOOR (issue: memory-fit override): a freeze-band block
        // (ratio >= 3x free RAM, here 30 needed / 10 free) must stand even
        // when the model is dismissed. The remember can never defeat the
        // guardrail on the dangerous case.
        let freeze = MemoryGate::Block {
            required_bytes: 30 * BYTES_PER_GIB,
            available_bytes: 10 * BYTES_PER_GIB,
        };
        assert_eq!(apply_dismissed_override(freeze, true), freeze);
        assert_eq!(apply_dismissed_override(freeze, false), freeze);
    }

    #[test]
    fn dismissed_override_end_to_end_bands() {
        let target = PathBuf::from("/blobs/target");
        let available = 10 * BYTES_PER_GIB;
        // Comfortable: well under the ceiling -> Proceed regardless of dismissed.
        let comfy = evaluate_load_gate(BYTES_PER_GIB, available, None, &target, &[], false);
        assert_eq!(apply_dismissed_override(comfy, false), MemoryGate::Proceed);

        // Mild band, the "squeeze it out" case: weights sized so required lands
        // just OVER available but far below the 3x freeze floor.
        // `estimate_required_bytes` adds RUNTIME_OVERHEAD_GB (2 GiB), so 9 GiB
        // weights -> 11 GiB required / 10 GiB available = ratio 1.10.
        let mild_weights = 9 * BYTES_PER_GIB;
        let mild = evaluate_load_gate(mild_weights, available, None, &target, &[], false);
        // Unforced + not dismissed blocks; dismissed rescues it.
        assert!(matches!(mild, MemoryGate::Block { .. }));
        assert_eq!(apply_dismissed_override(mild, false), mild);
        assert_eq!(apply_dismissed_override(mild, true), MemoryGate::Proceed);

        // Middle of the widened rememberable band: 18 GiB weights -> 20 GiB
        // required / 10 GiB available = ratio 2.00, still under the floor.
        let mid = evaluate_load_gate(18 * BYTES_PER_GIB, available, None, &target, &[], false);
        assert!(matches!(mid, MemoryGate::Block { .. }));
        assert_eq!(apply_dismissed_override(mid, true), MemoryGate::Proceed);

        // Freeze band: 40 GiB weights -> 42 GiB required / 10 GiB available =
        // ratio 4.20, at or above the 3x floor.
        let freeze = evaluate_load_gate(40 * BYTES_PER_GIB, available, None, &target, &[], false);
        assert!(matches!(freeze, MemoryGate::Block { .. }));
        // Dismissed still blocks in the freeze band.
        assert_eq!(apply_dismissed_override(freeze, true), freeze);
        // Forced proceeds upstream, and the override passes Proceed through.
        let forced = evaluate_load_gate(40 * BYTES_PER_GIB, available, None, &target, &[], true);
        assert_eq!(forced, MemoryGate::Proceed);
        assert_eq!(apply_dismissed_override(forced, true), MemoryGate::Proceed);
    }
}
