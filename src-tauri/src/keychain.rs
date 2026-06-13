//! Keychain integration for per-provider API key storage.
//!
//! API keys for `openai`-kind providers are stored in the macOS Keychain
//! under [`KEYCHAIN_SERVICE`]. The Keychain is the only place keys ever
//! live; they are never written to the TOML config and are never returned
//! to the frontend (only existence is queryable via [`has_provider_api_key`]).
//!
//! ## Extension points
//!
//! The [`SecretStore`] trait decouples business logic from the Keychain so
//! command handlers and callers in later tasks can be tested with
//! [`FakeSecretStore`] without touching the real user Keychain.

use std::sync::Arc;

// ─── Service constant ────────────────────────────────────────────────────────

/// Keychain service name under which per-provider API keys are stored.
/// Account = provider id. Stable: changing it orphans existing entries.
pub const KEYCHAIN_SERVICE: &str = "com.quietnode.thuki.provider-api-key";

// ─── Trait ───────────────────────────────────────────────────────────────────

pub trait SecretStore: Send + Sync + 'static {
    fn set(&self, provider_id: &str, secret: &str) -> Result<(), String>;
    fn get(&self, provider_id: &str) -> Result<Option<String>, String>;
    /// Deleting a missing entry is `Ok`.
    fn delete(&self, provider_id: &str) -> Result<(), String>;
}

// ─── keyring-backed implementation ───────────────────────────────────────────

/// macOS Keychain backend via the `keyring` crate. Thin wrapper: every method
/// body is a direct `keyring::Entry` call plus error mapping.
///
/// Not covered by the cargo coverage gate: this is a direct OS call with no
/// branching logic of its own; logic lives in callers tested with
/// [`FakeSecretStore`].
pub struct KeyringStore;

#[cfg_attr(coverage_nightly, coverage(off))]
impl SecretStore for KeyringStore {
    fn set(&self, provider_id: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, provider_id)
            .map_err(|e| e.to_string())?
            .set_password(secret)
            .map_err(|e| e.to_string())
    }

    fn get(&self, provider_id: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(KEYCHAIN_SERVICE, provider_id)
            .map_err(|e| e.to_string())?
            .get_password()
        {
            Ok(pw) => Ok(Some(pw)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn delete(&self, provider_id: &str) -> Result<(), String> {
        match keyring::Entry::new(KEYCHAIN_SERVICE, provider_id)
            .map_err(|e| e.to_string())?
            .delete_credential()
        {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

// ─── In-memory fake (tests only) ─────────────────────────────────────────────

/// In-memory [`SecretStore`] for unit tests. Available crate-wide during
/// `cargo test` so other modules' tests can construct it without touching the
/// real user Keychain.
#[cfg(test)]
pub(crate) struct FakeSecretStore {
    map: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

#[cfg(test)]
impl FakeSecretStore {
    pub(crate) fn new() -> Self {
        Self {
            map: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

#[cfg(test)]
impl SecretStore for FakeSecretStore {
    fn set(&self, provider_id: &str, secret: &str) -> Result<(), String> {
        self.map
            .lock()
            .unwrap()
            .insert(provider_id.to_string(), secret.to_string());
        Ok(())
    }

    fn get(&self, provider_id: &str) -> Result<Option<String>, String> {
        Ok(self.map.lock().unwrap().get(provider_id).cloned())
    }

    fn delete(&self, provider_id: &str) -> Result<(), String> {
        self.map.lock().unwrap().remove(provider_id);
        Ok(())
    }
}

// ─── Newtype wrapper for Tauri managed state ─────────────────────────────────

/// Newtype around `Arc<dyn SecretStore>` so Tauri's managed-state system can
/// hold the trait object. (`State<Arc<dyn Trait>>` fights the type system
/// because Tauri's `Manager::manage` requires `T: Any + Send + Sync`; wrapping
/// in a named newtype satisfies that bound cleanly.)
pub struct Secrets(pub Arc<dyn SecretStore>);

// ─── Input validation ────────────────────────────────────────────────────────

/// Pure, tested validation for `set_provider_api_key` inputs.
///
/// Returns `Err` when:
/// - `provider_id` is empty or longer than 128 bytes.
/// - `key` is empty or longer than 4096 bytes.
pub fn validate_key_input(provider_id: &str, key: &str) -> Result<(), String> {
    if provider_id.is_empty() {
        return Err("provider_id must not be empty".to_string());
    }
    if provider_id.len() > 128 {
        return Err("provider_id must be at most 128 bytes".to_string());
    }
    if key.is_empty() {
        return Err("key must not be empty".to_string());
    }
    if key.len() > 4096 {
        return Err("key must be at most 4096 bytes".to_string());
    }
    Ok(())
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Stores an API key for a provider in the macOS Keychain.
///
/// Validates inputs, then delegates to the managed [`SecretStore`].
/// The secret value never crosses the IPC boundary in any response.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn set_provider_api_key(
    provider_id: String,
    key: String,
    store: tauri::State<'_, Secrets>,
) -> Result<(), String> {
    validate_key_input(&provider_id, &key)?;
    store.0.set(&provider_id, &key)
}

/// Removes an API key for a provider from the macOS Keychain.
///
/// Deleting a missing entry succeeds silently.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn clear_provider_api_key(
    provider_id: String,
    store: tauri::State<'_, Secrets>,
) -> Result<(), String> {
    store.0.delete(&provider_id)
}

/// Returns `true` if an API key exists for the provider, `false` otherwise.
///
/// The secret value is never included in the response.
#[cfg_attr(coverage_nightly, coverage(off))]
#[cfg_attr(not(coverage), tauri::command)]
pub fn has_provider_api_key(
    provider_id: String,
    store: tauri::State<'_, Secrets>,
) -> Result<bool, String> {
    store.0.get(&provider_id).map(|o| o.is_some())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // Service name is load-bearing: changing it orphans existing Keychain entries.
    // This test makes any accidental rename visible in review.
    #[test]
    fn service_name_is_stable() {
        assert_eq!(KEYCHAIN_SERVICE, "com.quietnode.thuki.provider-api-key");
    }

    #[test]
    fn validate_key_input_rejects_empty_and_oversize() {
        // empty provider_id
        assert!(validate_key_input("", "somekey").is_err());
        // empty key
        assert!(validate_key_input("provider1", "").is_err());
        // provider_id exactly 128 bytes: ok
        let id_128 = "a".repeat(128);
        assert!(validate_key_input(&id_128, "somekey").is_ok());
        // provider_id 129 bytes: err
        let id_129 = "a".repeat(129);
        assert!(validate_key_input(&id_129, "somekey").is_err());
        // key exactly 4096 bytes: ok
        let key_4096 = "k".repeat(4096);
        assert!(validate_key_input("provider1", &key_4096).is_ok());
        // key 4097 bytes: err
        let key_4097 = "k".repeat(4097);
        assert!(validate_key_input("provider1", &key_4097).is_err());
    }

    #[test]
    fn fake_store_set_get_delete_roundtrip() {
        let store = FakeSecretStore::new();

        // set then get returns the value
        store.set("prov-a", "sk-secret123").unwrap();
        assert_eq!(
            store.get("prov-a").unwrap(),
            Some("sk-secret123".to_string())
        );

        // overwrite works
        store.set("prov-a", "sk-new").unwrap();
        assert_eq!(store.get("prov-a").unwrap(), Some("sk-new".to_string()));

        // delete removes the entry
        store.delete("prov-a").unwrap();
        assert_eq!(store.get("prov-a").unwrap(), None);

        // has-key logic (mirrors the command body)
        assert!(!store.get("prov-a").unwrap().is_some());
        store.set("prov-b", "key").unwrap();
        assert!(store.get("prov-b").unwrap().is_some());
    }

    #[test]
    fn fake_delete_missing_is_ok() {
        let store = FakeSecretStore::new();
        // deleting an entry that was never set must succeed
        assert!(store.delete("nonexistent").is_ok());
    }
}
