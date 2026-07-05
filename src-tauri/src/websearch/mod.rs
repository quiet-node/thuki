//! Built-in zero-setup web search.
//!
//! Replaces the old Docker `/search` pipeline with an invisible, model-decided
//! search that runs entirely on the device: a grammar-constrained pre-pass
//! decides per message whether the web is needed, keyless sources are fetched
//! through the SSRF-safe [`crate::net`] transport, and a single writer call
//! answers with numbered citations.
//!
//! The stages are built as independent, injectable units so the orchestrator's
//! decision logic is unit-testable without a live model or network:
//! - [`prepass`] — the `no｜cached｜web` trigger and query rewrite.

pub mod prepass;
