//! SSRF-safe outbound HTTP for the built-in web search stack.
//!
//! A single injectable [`transport::HttpTransport`] is shared by the search
//! engine client, the vertical API clients, and the page fetcher, so every
//! outbound request is subject to the same [`ssrf`] guard, proxy policy,
//! redirect cap, timeout, and response-size cap. Callers depend on the trait,
//! not the concrete reqwest backend, which keeps their logic testable without
//! a network.

pub mod reachability;
pub mod ssrf;
pub mod transport;
