//! Offline fast-fail: an injectable reachability signal, raced against the real
//! engine requests so a fully offline turn reaches the honest "can't reach the
//! web" disclosure in about a second instead of after every engine's stacked
//! connect + request timeouts.
//!
//! Two rules shape this module, and both exist to protect the user from being
//! told they are offline when they are not (a lie is worse than a stall):
//!
//! - **Never a pre-flight gate.** Requests are never blocked on a reachability
//!   check (explicit Apple guidance, WWDC 2018 session 714: a reachability
//!   verdict is a hint about the past, not a promise about the next request).
//!   The signal is RACED against the real fetch by
//!   [`crate::websearch::orchestrator`]; if the real fetch returns first it wins,
//!   always.
//! - **Only a positive unreachable signal counts.** A probe that times out, or
//!   that cannot make up its mind, resolves to [`ReachabilityVerdict::Unknown`]
//!   and NEVER short-circuits. An inconclusive probe on a slow hotel Wi-Fi is
//!   exactly the input that would otherwise produce a false "you are offline".
//!
//! Probe target: DNS resolution of EVERY keyless SERP host the engine tier is
//! about to contact (derived from `crate::websearch::engine::SERP_ENDPOINTS`, so
//! adding an engine automatically extends the probe). Chosen deliberately, and
//! the choice is the whole false-positive story:
//!
//! - **All hosts, not one.** The tier races several engines and one live engine
//!   is enough to answer the turn, so ANY host resolving means Reachable. A
//!   single-host probe would report "no internet" on a network that merely
//!   DNS-blocks that one engine (a documented corporate, school, ISP, and
//!   national-filter configuration for DuckDuckGo specifically), while Mojeek
//!   would have answered the query perfectly well. Probing one engine and
//!   speaking for the whole round is exactly the confident lie this module
//!   exists to prevent.
//! - It contacts no host the search pipeline was not already going to contact,
//!   so a privacy-first app grows no new third-party liveness endpoint (no
//!   `captive.apple.com`, no `google.com`, no telemetry-ish beacon).
//! - It needs no new dependency: `tokio::net::lookup_host` is already in the
//!   tree and is the same resolution the real requests perform through the
//!   transport's pinning resolver.
//! - It cannot be more pessimistic than the round it races: an Unreachable
//!   verdict needs every engine's name to fail to resolve, and a name that will
//!   not resolve is a request that cannot connect. The reverse (a resolver
//!   answering from cache while the link is down) only costs us the speedup and
//!   falls back to the old timeout behaviour, which is the safe direction to be
//!   wrong in.
//!
//! Captive-portal detection is explicitly out of scope and is not built here.

use async_trait::async_trait;

use crate::config::defaults::{OFFLINE_SHORTCIRCUIT_WINDOW_MS, REACHABILITY_PROBE_TIMEOUT_MS};

/// What resolving ONE engine host produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostProbe {
    /// The name resolved to at least one address.
    Resolved,
    /// The resolver refused or errored, or answered with no address at all.
    Failed,
}

/// The engine hosts the probe resolves, derived from the SERP endpoints the
/// engine tier actually requests (`crate::websearch::engine::SERP_ENDPOINTS`)
/// rather than restated as literals here, so a third engine cannot be added
/// while the probe silently keeps testing only the old two. An endpoint that
/// somehow carries no host is skipped rather than treated as a failure: it is a
/// compile-time-fixed constant, and a malformed one must never manufacture
/// offline evidence.
pub(crate) fn probe_hosts() -> Vec<&'static str> {
    crate::websearch::engine::SERP_ENDPOINTS
        .iter()
        .filter_map(|endpoint| endpoint.split('/').nth(2))
        .collect()
}

/// Reduces the per-host resolution outcomes into the turn's verdict. Pure, so
/// the rule that actually protects the user is unit-tested without a resolver:
///
/// - ANY host resolved: [`ReachabilityVerdict::Reachable`]. One live engine can
///   answer the query, so the turn must not be short-circuited.
/// - EVERY host failed (and at least one was probed):
///   [`ReachabilityVerdict::Unreachable`]. No engine's name resolves, so no
///   engine request can connect.
/// - Nothing conclusive to reduce (no hosts): [`ReachabilityVerdict::Unknown`].
///   Hosts that never answer inside the deadline never reach this function at
///   all: [`offline_cutoff`] times the whole probe out and calls it `Unknown`.
pub(crate) fn reduce_verdict(outcomes: &[HostProbe]) -> ReachabilityVerdict {
    if outcomes.contains(&HostProbe::Resolved) {
        ReachabilityVerdict::Reachable
    } else if outcomes.is_empty() {
        ReachabilityVerdict::Unknown
    } else {
        ReachabilityVerdict::Unreachable
    }
}

/// What a reachability probe concluded. Deliberately three-valued: "not proven
/// reachable" is NOT the same as "proven unreachable", and only the latter may
/// ever short-circuit a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReachabilityVerdict {
    /// The probe resolved: the network answered, so the real fetch gets its
    /// full budget.
    Reachable,
    /// The probe positively failed: the device has no usable path to the search
    /// hosts. The only verdict that can short-circuit.
    Unreachable,
    /// The probe timed out or produced no usable answer. Treated as "no
    /// evidence", never as offline.
    Unknown,
}

/// The injectable reachability signal (same pattern as
/// [`crate::net::transport::HttpTransport`] and `crate::keychain::SecretStore`):
/// production resolves DNS, tests script a verdict, so the pure decision logic
/// in [`offline_cutoff`] is covered without a syscall or a network.
#[async_trait]
pub trait Reachability: Send + Sync {
    /// Probes the network once and reports what it could prove.
    async fn probe(&self) -> ReachabilityVerdict;
}

/// Resolves every host from [`probe_hosts`] concurrently through the system
/// resolver and reduces the results with [`reduce_verdict`]. Coverage-excluded:
/// the `lookup_host` calls are thin OS glue (the same precedent as
/// [`crate::net::transport::ReqwestTransport`]); the host derivation and the
/// verdict rule, which are the parts that can lie to a user, are pure and tested
/// directly.
///
/// A per-host resolver error is [`HostProbe::Failed`], not evidence of a working
/// link, because the real request resolves the same name through the same system
/// resolver: a name that will not resolve is a request that cannot connect. It
/// takes EVERY engine host failing to reach an Unreachable verdict, and even
/// then [`offline_cutoff`] only short-circuits when the real requests have ALSO
/// failed to return within the grace window.
pub struct DnsReachability;

#[cfg_attr(coverage_nightly, coverage(off))]
#[async_trait]
impl Reachability for DnsReachability {
    async fn probe(&self) -> ReachabilityVerdict {
        // Port 443: `lookup_host` wants a port, and the search hosts are HTTPS.
        // The addresses themselves are discarded; only whether the name resolved
        // matters here (the real request re-resolves and SSRF-screens its own).
        let lookups = probe_hosts().into_iter().map(|host| async move {
            match tokio::net::lookup_host((host, 443)).await {
                Ok(mut addrs) => match addrs.next() {
                    Some(_) => HostProbe::Resolved,
                    // Resolved to nothing at all: no usable address for this engine.
                    None => HostProbe::Failed,
                },
                Err(_) => HostProbe::Failed,
            }
        });
        let outcomes = futures_util::future::join_all(lookups).await;
        reduce_verdict(&outcomes)
    }
}

/// The offline short-circuit signal: a future that resolves ONLY when the turn
/// should stop waiting on the engines and go straight to the honest "can't reach
/// the web" path, and otherwise stays pending forever so the caller's
/// `tokio::select!` simply keeps waiting on the real fetch.
///
/// Sequence: probe under a [`REACHABILITY_PROBE_TIMEOUT_MS`] deadline; a
/// timeout or any non-[`ReachabilityVerdict::Unreachable`] verdict yields a
/// never-resolving future (no short-circuit, ever); a proven-unreachable verdict
/// waits out the remainder of the [`OFFLINE_SHORTCIRCUIT_WINDOW_MS`] grace
/// window before resolving, so a real fetch that is merely slow to start still
/// gets that window to come back and win the race.
///
/// The caller races this against the live engine round with a biased
/// `tokio::select!` whose fetch arm is polled first, so a fetch that completes
/// wins even on a tie, and taking this branch DROPS the in-flight requests: a
/// late-returning fetch can therefore never surface a result that contradicts
/// the disclosure the user was already shown.
pub(crate) async fn offline_cutoff(reachability: &dyn Reachability) {
    let started = tokio::time::Instant::now();
    let probe = tokio::time::timeout(
        std::time::Duration::from_millis(REACHABILITY_PROBE_TIMEOUT_MS),
        reachability.probe(),
    )
    .await;
    // A probe that did not finish in time proves nothing: it must read as
    // `Unknown`, never as offline (see the module docs).
    let verdict = probe.unwrap_or(ReachabilityVerdict::Unknown);
    if verdict != ReachabilityVerdict::Unreachable {
        std::future::pending::<()>().await;
    }
    // Wait out whatever is left of the grace window. Saturating, not checked:
    // the probe deadline is strictly below the window, so the remainder is
    // normally positive, and a probe that somehow overran it should short-circuit
    // immediately rather than take an unreachable branch.
    let window = std::time::Duration::from_millis(OFFLINE_SHORTCIRCUIT_WINDOW_MS);
    tokio::time::sleep(window.saturating_sub(started.elapsed())).await;
}

/// Scriptable [`Reachability`] for unit tests: replays a fixed verdict, or hangs
/// forever so the probe-timeout path is exercised without a real resolver.
/// Available crate-wide during `cargo test` so the orchestrator's pipeline tests
/// can drive an offline turn.
#[cfg(test)]
pub(crate) struct FakeReachability {
    verdict: Option<ReachabilityVerdict>,
}

#[cfg(test)]
impl FakeReachability {
    /// A probe that immediately reports `verdict`.
    pub(crate) fn returning(verdict: ReachabilityVerdict) -> Self {
        Self {
            verdict: Some(verdict),
        }
    }

    /// A probe that never answers, so [`offline_cutoff`] hits its deadline and
    /// must resolve to [`ReachabilityVerdict::Unknown`].
    pub(crate) fn hanging() -> Self {
        Self { verdict: None }
    }
}

#[cfg(test)]
#[async_trait]
impl Reachability for FakeReachability {
    async fn probe(&self) -> ReachabilityVerdict {
        match self.verdict {
            Some(verdict) => verdict,
            None => std::future::pending().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── probe_hosts / reduce_verdict ──────────────────────────────────────────

    #[test]
    fn probe_hosts_covers_every_serp_engine_endpoint() {
        let hosts = probe_hosts();
        // Derived from the engine tier's own endpoint list, so the probe grows
        // with `ENGINES` instead of drifting behind it.
        assert_eq!(hosts.len(), crate::websearch::engine::SERP_ENDPOINTS.len());
        assert!(hosts.contains(&"html.duckduckgo.com"));
        assert!(hosts.contains(&"www.mojeek.com"));
    }

    #[test]
    fn every_host_failing_is_unreachable() {
        assert_eq!(
            reduce_verdict(&[HostProbe::Failed, HostProbe::Failed]),
            ReachabilityVerdict::Unreachable
        );
    }

    /// The regression this reduction exists for: a network that DNS-blocks
    /// DuckDuckGo (a documented corporate, school, ISP, and national-filter
    /// configuration) but resolves Mojeek fine is ONLINE, and Mojeek alone can
    /// answer the query. It must never be told it has no internet.
    #[test]
    fn ddg_blocked_but_mojeek_resolving_is_reachable_not_offline() {
        assert_eq!(
            reduce_verdict(&[HostProbe::Failed, HostProbe::Resolved]),
            ReachabilityVerdict::Reachable
        );
        // Order-independent: whichever engine is the blocked one.
        assert_eq!(
            reduce_verdict(&[HostProbe::Resolved, HostProbe::Failed]),
            ReachabilityVerdict::Reachable
        );
    }

    #[test]
    fn no_host_outcomes_at_all_is_unknown() {
        assert_eq!(reduce_verdict(&[]), ReachabilityVerdict::Unknown);
    }

    // ── offline_cutoff ────────────────────────────────────────────────────────

    /// Runs [`offline_cutoff`] against `reachability` under a generous virtual
    /// deadline and reports the virtual time it took, or `None` if it never
    /// resolved (the no-short-circuit contract).
    async fn cutoff_after(reachability: &dyn Reachability) -> Option<std::time::Duration> {
        let started = tokio::time::Instant::now();
        tokio::time::timeout(
            std::time::Duration::from_secs(60),
            offline_cutoff(reachability),
        )
        .await
        .ok()
        .map(|()| started.elapsed())
    }

    #[tokio::test(start_paused = true)]
    async fn unreachable_probe_short_circuits_at_the_window() {
        let elapsed = cutoff_after(&FakeReachability::returning(
            ReachabilityVerdict::Unreachable,
        ))
        .await
        .expect("a proven-unreachable probe must short-circuit");
        assert_eq!(
            elapsed,
            std::time::Duration::from_millis(OFFLINE_SHORTCIRCUIT_WINDOW_MS)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn reachable_probe_never_short_circuits() {
        // The false-positive guard: a working network must never be told it is
        // offline, no matter how slow the real fetch is.
        assert!(
            cutoff_after(&FakeReachability::returning(ReachabilityVerdict::Reachable))
                .await
                .is_none()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn unknown_probe_never_short_circuits() {
        assert!(
            cutoff_after(&FakeReachability::returning(ReachabilityVerdict::Unknown))
                .await
                .is_none()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn timed_out_probe_is_unknown_and_never_short_circuits() {
        assert!(cutoff_after(&FakeReachability::hanging()).await.is_none());
    }
}
