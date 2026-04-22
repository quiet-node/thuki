//! Shared retry and error-classification helpers for the agentic /search loop.
//!
//! Used by the router, SearXNG, reader, and judge callers to enforce the
//! "single retry on transient failures, no retry on semantic failures" rule
//! locked in the design doc. Keeps call sites free of per-module retry boilerplate.

use std::future::Future;
use std::time::Duration;

use tokio::time::sleep;

/// Run the given async operation. On the first failure, wait `delay`, then
/// try exactly once more. Returns the final `Result<T, E>` from whichever
/// attempt ran last. The operation may be called at most twice.
///
/// No exponential backoff: the semantics we want are "hiccup recovery", not
/// generic retry-with-backoff. If both attempts fail, propagate the error.
pub async fn retry_once<F, Fut, T, E>(delay: Duration, mut op: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    match op().await {
        Ok(v) => Ok(v),
        Err(_first) => {
            sleep(delay).await;
            op().await
        }
    }
}

/// Classify an error message as transient (worth a single retry).
///
/// Matches lowercase substrings typical of reqwest / hyper / io errors for
/// connection faults, timeouts, DNS failures, and broken pipes. Semantic
/// errors (`404`, `400`, `parse error`) are NOT transient.
pub fn is_transient_connect_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("connection refused")
        || m.contains("connection reset")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("dns")
        || m.contains("broken pipe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn retry_once_succeeds_on_second_attempt() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result = retry_once(Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                let attempt = c.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Err::<u32, &'static str>("boom")
                } else {
                    Ok(7)
                }
            }
        })
        .await;
        assert_eq!(result, Ok(7));
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn retry_once_returns_second_error_on_double_failure() {
        let result = retry_once(Duration::from_millis(1), || async {
            Err::<u32, &'static str>("nope")
        })
        .await;
        assert_eq!(result, Err("nope"));
    }

    #[tokio::test]
    async fn retry_once_skips_retry_on_first_success() {
        let counter = Arc::new(AtomicU32::new(0));
        let c = counter.clone();
        let result: Result<u32, &'static str> = retry_once(Duration::from_millis(1), || {
            let c = c.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(42)
            }
        })
        .await;
        assert_eq!(result, Ok(42));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn transient_classifier_matches_connect_errors() {
        assert!(is_transient_connect_error("Connection refused"));
        assert!(is_transient_connect_error("operation timed out"));
        assert!(is_transient_connect_error("dns error"));
        assert!(is_transient_connect_error("connection reset by peer"));
        assert!(is_transient_connect_error("broken pipe"));
    }

    #[test]
    fn transient_classifier_rejects_semantic_errors() {
        assert!(!is_transient_connect_error("404 Not Found"));
        assert!(!is_transient_connect_error("parse error"));
        assert!(!is_transient_connect_error("invalid json"));
    }
}
