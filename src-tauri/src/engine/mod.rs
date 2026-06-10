//! Built-in inference engine management.
//!
//! Thuki bundles a single `llama-server` sidecar and manages its lifecycle:
//! at most one engine process exists, never two models are resident, and a
//! model (or context-size) switch always kills the old process and waits for
//! a confirmed exit before spawning the new one. This module hosts the pieces
//! of that lifecycle; the pure residency state machine lives in [`state`].

pub mod state;
