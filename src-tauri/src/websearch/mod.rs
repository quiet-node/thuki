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
//! - [`engine`] — keyless search-engine scraping with rotation.
//! - [`fetch`] — concurrent page fetch + readability extraction.
//! - [`rank`] — chunking + BM25 extractive filter behind a `Scorer` seam.
//! - [`assemble`] — group ranked chunks into budgeted numbered source blocks.
//! - [`writer`] — writer prompt assembly with prompt-injection defenses.
//! - [`orchestrator`] — the fixed pipeline tying the stages together.

pub mod assemble;
pub mod engine;
pub mod fetch;
pub mod orchestrator;
pub mod prepass;
pub mod rank;
pub mod writer;

/// The registration host of a URL, or an empty string when it does not parse.
/// Shared by the engine's per-domain result cap and the writer's per-source
/// trust label.
pub(crate) fn domain_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn domain_of_extracts_host_or_empty() {
        assert_eq!(super::domain_of("https://sub.example.com/path"), "sub.example.com");
        assert_eq!(super::domain_of("not a url"), "");
    }
}
