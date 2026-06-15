//! Config migration helpers shared by the loader (TOML-shape migration) and
//! startup orchestration (SQLite active-model fold-in). Kept pure so both
//! halves are unit-tested without a Tauri app or a real SQLite connection.

use super::defaults::PROVIDER_KIND_OLLAMA;
use super::schema::AppConfig;

/// Attaches a legacy SQLite `active_model` onto the active provider's `model`
/// field when that provider is Ollama-kind and has no model yet. The legacy
/// slug is by definition an Ollama model name, so it never attaches to a
/// provider of any other kind. Returns true if it mutated the config (so
/// startup can decide whether to persist). No-op when `legacy` is
/// empty/whitespace, the active provider is not Ollama-kind, or it already
/// has a model.
pub fn attach_legacy_active_model(config: &mut AppConfig, legacy: Option<&str>) -> bool {
    let Some(model) = legacy.map(str::trim).filter(|m| !m.is_empty()) else {
        return false;
    };
    let active_id = config.inference.active_provider.clone();
    if let Some(provider) = config
        .inference
        .providers
        .iter_mut()
        .find(|p| p.id == active_id)
    {
        if provider.kind != PROVIDER_KIND_OLLAMA {
            return false;
        }
        if provider.model.trim().is_empty() {
            provider.model = model.to_string();
            return true;
        }
    }
    false
}

/// True if the given config TOML text already carries a non-empty
/// `[[inference.providers]]` array (i.e. it is the new shape). Used by startup
/// to decide whether to perform the one-time old → new shape upgrade write.
/// Unparseable input is treated as "not the new shape".
pub fn toml_has_providers(toml_text: &str) -> bool {
    toml_text
        .parse::<toml::Table>()
        .ok()
        .and_then(|table| {
            table
                .get("inference")?
                .as_table()?
                .get("providers")?
                .as_array()
                .map(|a| !a.is_empty())
        })
        .unwrap_or(false)
}
