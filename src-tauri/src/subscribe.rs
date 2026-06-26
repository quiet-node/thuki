//! Optional email-capture for the "Help shape Thuki" ask (onboarding roadmap
//! screen + Settings ▸ About).
//!
//! The user, on an explicit click, may leave their email so the founder can
//! reach out and shape the roadmap. Thuki POSTs only `{ email, source }` to a
//! public proxy ([`DEFAULT_SUBSCRIBE_ENDPOINT`]); the proxy holds the
//! email-service key. No secret ever lives in the app, and nothing is sent
//! without that click, preserving the no-phone-home posture.
//!
//! The contract is intentionally tiny:
//! `200` (including already-subscribed) → success; any `4xx`/`5xx` or network
//! failure → a generic friendly error. The server's error body is never
//! surfaced to the UI (trust boundary): the `Err(String)` is effectively just
//! a failure discriminant the frontend turns into its own generic line.

use serde::Serialize;

use crate::config::defaults::DEFAULT_SUBSCRIBE_ENDPOINT;

/// JSON body sent to the subscribe proxy. `source` lets the proxy attribute
/// sign-ups to the app vs. the landing page.
#[derive(Serialize)]
struct SubscribeRequest<'a> {
    email: &'a str,
    source: &'a str,
}

/// Server-side (defense-in-depth) email shape check. Deliberately mirrors the
/// frontend regex `^[^\s@]+@[^\s@]+\.[^\s@]+$` without pulling a regex
/// dependency, and is no stricter than it: any address the frontend accepts
/// must pass here too, so a client-validated email can never 400 on shape.
fn is_valid_email(email: &str) -> bool {
    if email.is_empty() || email.chars().any(char::is_whitespace) {
        return false;
    }
    // Exactly one `@` with a non-empty local part. `splitn(2, '@')` keeps any
    // further `@` inside `domain`, which the dot check below then rejects.
    let mut parts = email.splitn(2, '@');
    let local = parts.next().unwrap_or("");
    let domain = parts.next().unwrap_or("");
    if local.is_empty() || domain.contains('@') {
        return false;
    }
    // Domain needs a dot with a non-empty host and TLD on either side.
    match domain.rsplit_once('.') {
        Some((host, tld)) => !host.is_empty() && !tld.is_empty(),
        None => false,
    }
}

/// Validate, then POST `{ email, source: "app" }` to `endpoint` and map the
/// response. A `2xx` (including an already-subscribed response) is success;
/// an invalid address, a non-`2xx` status, or any transport failure is a
/// generic `Err`. The `endpoint` is a parameter purely so tests can target a
/// mock server; production callers pass [`DEFAULT_SUBSCRIBE_ENDPOINT`].
pub async fn post_subscribe(
    client: &reqwest::Client,
    endpoint: &str,
    email: &str,
) -> Result<(), String> {
    let email = email.trim();
    if !is_valid_email(email) {
        return Err("Please enter a valid email address.".to_string());
    }

    let response = client
        .post(endpoint)
        .json(&SubscribeRequest {
            email,
            source: "app",
        })
        .send()
        .await
        .map_err(|_| "Couldn't reach the network. Please try again.".to_string())?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err("Something went wrong. Please try again later.".to_string())
    }
}

/// Tauri command: subscribe the given email via the baked-in proxy endpoint.
/// Thin I/O wrapper (resolve the managed HTTP client + the fixed endpoint,
/// delegate to [`post_subscribe`]); the logic and every response mapping are
/// covered through `post_subscribe`'s tests.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub async fn subscribe_email(
    email: String,
    client: tauri::State<'_, reqwest::Client>,
) -> Result<(), String> {
    post_subscribe(client.inner(), DEFAULT_SUBSCRIBE_ENDPOINT, &email).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn is_valid_email_accepts_well_formed_addresses() {
        assert!(is_valid_email("founder@thuki.app"));
        assert!(is_valid_email("a.b+tag@sub.example.co.uk"));
    }

    #[test]
    fn is_valid_email_rejects_malformed_addresses() {
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("plainaddress"));
        assert!(!is_valid_email("no-at-sign.com"));
        assert!(!is_valid_email("@nolocal.com"));
        assert!(!is_valid_email("nodomain@"));
        assert!(!is_valid_email("two@@at.com"));
        assert!(!is_valid_email("user@nodot"));
        assert!(!is_valid_email("user@.com"));
        assert!(!is_valid_email("user@domain."));
        assert!(!is_valid_email("has space@thuki.app"));
    }

    #[tokio::test]
    async fn post_subscribe_returns_ok_on_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/subscribe"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/subscribe", server.uri());
        let result = post_subscribe(&client, &endpoint, "founder@thuki.app").await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn post_subscribe_treats_already_subscribed_200_as_ok() {
        let server = MockServer::start().await;
        // The proxy returns 200 with `{ok:true}` even when the address is
        // already on the list; that must read as success, not an error.
        Mock::given(method("POST"))
            .and(path("/api/subscribe"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/subscribe", server.uri());
        let result = post_subscribe(&client, &endpoint, "  founder@thuki.app  ").await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn post_subscribe_sends_trimmed_email_and_app_source_as_json() {
        let server = MockServer::start().await;
        // The body matcher asserts the exact request shape: a trimmed email and
        // `source: "app"`, posted as JSON.
        Mock::given(method("POST"))
            .and(path("/api/subscribe"))
            .and(body_json(
                serde_json::json!({"email": "founder@thuki.app", "source": "app"}),
            ))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/subscribe", server.uri());
        let result = post_subscribe(&client, &endpoint, "  founder@thuki.app  ").await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn post_subscribe_rejects_invalid_email_without_calling_the_server() {
        let server = MockServer::start().await;
        // No mock is mounted: if the function called out, the request would be
        // recorded. We assert it never reached the network.
        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/subscribe", server.uri());
        let result = post_subscribe(&client, &endpoint, "not-an-email").await;
        assert_eq!(
            result,
            Err("Please enter a valid email address.".to_string())
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn post_subscribe_maps_4xx_to_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/subscribe"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"ok": false, "error": "bad request"})),
            )
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/subscribe", server.uri());
        let result = post_subscribe(&client, &endpoint, "founder@thuki.app").await;
        assert_eq!(
            result,
            Err("Something went wrong. Please try again later.".to_string())
        );
    }

    #[tokio::test]
    async fn post_subscribe_maps_5xx_to_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/subscribe"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let client = reqwest::Client::new();
        let endpoint = format!("{}/api/subscribe", server.uri());
        let result = post_subscribe(&client, &endpoint, "founder@thuki.app").await;
        assert_eq!(
            result,
            Err("Something went wrong. Please try again later.".to_string())
        );
    }

    #[tokio::test]
    async fn post_subscribe_maps_network_failure_to_error() {
        let client = reqwest::Client::new();
        // Port 1 is always refused on localhost, forcing a transport error.
        let result = post_subscribe(
            &client,
            "http://127.0.0.1:1/api/subscribe",
            "founder@thuki.app",
        )
        .await;
        assert_eq!(
            result,
            Err("Couldn't reach the network. Please try again.".to_string())
        );
    }
}
