//! Config file load, parse, and resolve-to-defaults pipeline.
//!
//! Flow:
//!   1. Read file from `path`.
//!        - Missing (NotFound)     -> seed defaults, write them, return defaults.
//!        - Permission / I/O error -> log, return defaults (no seed attempt).
//!        - Ok(contents)           -> fall through to parse.
//!   2. Parse TOML.
//!        - Parse error -> rename file to `<name>.corrupt-<ts>`, seed defaults.
//!   3. Check schema_version.
//!        - Newer than supported -> rename, seed defaults.
//!        - Unknown older version -> rename, seed defaults.
//!   4. Resolve (empties -> defaults, out-of-bounds -> defaults, compose appendix).
//!
//! All "rename and reseed" paths are non-fatal. Only first-run seed failure is
//! fatal (the app cannot boot in a writable-hostile environment and the user
//! cannot fix that from the UI).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::defaults::{
    BOUNDS_COLLAPSED_HEIGHT, BOUNDS_COOLDOWN_MS, BOUNDS_DOUBLE_TAP_WINDOW_MS,
    BOUNDS_HIDE_COMMIT_DELAY_MS, BOUNDS_MAX_CHAT_HEIGHT, BOUNDS_OVERLAY_WIDTH,
    BOUNDS_QUOTE_MAX_CONTEXT_LENGTH, BOUNDS_QUOTE_MAX_DISPLAY_CHARS,
    BOUNDS_QUOTE_MAX_DISPLAY_LINES, CURRENT_SCHEMA_VERSION, DEFAULT_COLLAPSED_HEIGHT,
    DEFAULT_COOLDOWN_MS, DEFAULT_DOUBLE_TAP_WINDOW_MS, DEFAULT_HIDE_COMMIT_DELAY_MS,
    DEFAULT_MAX_CHAT_HEIGHT, DEFAULT_MODEL_NAME, DEFAULT_OLLAMA_URL, DEFAULT_OVERLAY_WIDTH,
    DEFAULT_QUOTE_MAX_CONTEXT_LENGTH, DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
    DEFAULT_QUOTE_MAX_DISPLAY_LINES, DEFAULT_SYSTEM_PROMPT_BASE, SLASH_COMMAND_PROMPT_APPENDIX,
};
use super::error::ConfigError;
use super::schema::AppConfig;
use super::writer::atomic_write;

/// Loads the configuration from the given path, applying every recovery rule
/// described in the module doc. Returns a fully-resolved, validated `AppConfig`
/// on success. Returns `Err(ConfigError::SeedFailed)` only if the file was
/// missing and the default seed write failed.
pub fn load_from_path(path: &Path) -> Result<AppConfig, ConfigError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => load_from_contents(path, &contents),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => seed_defaults(path),
        Err(source) => {
            eprintln!(
                "thuki: [config] cannot read {}: {source}. using in-memory defaults",
                path.display()
            );
            let mut config = AppConfig::default();
            resolve(&mut config);
            Ok(config)
        }
    }
}

fn load_from_contents(path: &Path, contents: &str) -> Result<AppConfig, ConfigError> {
    match toml::from_str::<AppConfig>(contents) {
        Ok(mut config) => match validate_schema_version(config.schema_version) {
            Ok(()) => {
                resolve(&mut config);
                Ok(config)
            }
            Err(e) => {
                eprintln!(
                    "thuki: [config] {e}: renaming and reseeding defaults at {}",
                    path.display()
                );
                rename_corrupt(path);
                seed_defaults(path)
            }
        },
        Err(parse_err) => {
            eprintln!(
                "thuki: [config] parse error at {}: {parse_err}. renaming and reseeding defaults",
                path.display()
            );
            rename_corrupt(path);
            seed_defaults(path)
        }
    }
}

/// Writes the compiled defaults to `path` and returns the resolved `AppConfig`.
/// This is the first-run path; any failure here is fatal and surfaced to the
/// caller (`lib.rs` shows a dialog and quits).
fn seed_defaults(path: &Path) -> Result<AppConfig, ConfigError> {
    let mut config = AppConfig::default();
    resolve(&mut config);
    atomic_write(path, &config).map_err(|source| ConfigError::SeedFailed {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(config)
}

/// Renames a corrupt or incompatible file to `<path>.corrupt-<unix_ts>`.
/// Best-effort; failures are logged but do not block the reseed.
fn rename_corrupt(path: &Path) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut target = path.as_os_str().to_os_string();
    target.push(format!(".corrupt-{ts}"));
    let target: PathBuf = target.into();
    if let Err(e) = std::fs::rename(path, &target) {
        eprintln!(
            "thuki: [config] could not rename corrupt file {}: {e}",
            path.display()
        );
    }
}

fn validate_schema_version(found: u32) -> Result<(), ConfigError> {
    if found == CURRENT_SCHEMA_VERSION {
        Ok(())
    } else if found > CURRENT_SCHEMA_VERSION {
        Err(ConfigError::TooNew {
            found,
            supported: CURRENT_SCHEMA_VERSION,
        })
    } else {
        Err(ConfigError::NoMigrationYet { found })
    }
}

/// Resolves empty strings to compiled defaults, clamps out-of-bounds numerics,
/// and composes the system prompt appendix into `prompt.resolved_system`.
/// After this runs, every `AppConfig` field holds a usable value.
pub(crate) fn resolve(config: &mut AppConfig) {
    // Model section: empty available list or empty/whitespace entries -> default.
    let cleaned: Vec<String> = config
        .model
        .available
        .iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect();
    config.model.available = if cleaned.is_empty() {
        vec![DEFAULT_MODEL_NAME.to_string()]
    } else {
        cleaned
    };
    if config.model.ollama_url.trim().is_empty() {
        config.model.ollama_url = DEFAULT_OLLAMA_URL.to_string();
    }

    // Prompt section: empty base -> built-in. Compose resolved_system.
    let base = if config.prompt.system.trim().is_empty() {
        DEFAULT_SYSTEM_PROMPT_BASE
    } else {
        &config.prompt.system
    };
    config.prompt.resolved_system = compose_system_prompt(base, SLASH_COMMAND_PROMPT_APPENDIX);

    // Window section.
    clamp_f64(
        &mut config.window.overlay_width,
        BOUNDS_OVERLAY_WIDTH,
        DEFAULT_OVERLAY_WIDTH,
        "window.overlay_width",
    );
    clamp_f64(
        &mut config.window.collapsed_height,
        BOUNDS_COLLAPSED_HEIGHT,
        DEFAULT_COLLAPSED_HEIGHT,
        "window.collapsed_height",
    );
    clamp_f64(
        &mut config.window.max_chat_height,
        BOUNDS_MAX_CHAT_HEIGHT,
        DEFAULT_MAX_CHAT_HEIGHT,
        "window.max_chat_height",
    );
    clamp_u64(
        &mut config.window.hide_commit_delay_ms,
        BOUNDS_HIDE_COMMIT_DELAY_MS,
        DEFAULT_HIDE_COMMIT_DELAY_MS,
        "window.hide_commit_delay_ms",
    );

    // Activation section.
    clamp_u64(
        &mut config.activation.double_tap_window_ms,
        BOUNDS_DOUBLE_TAP_WINDOW_MS,
        DEFAULT_DOUBLE_TAP_WINDOW_MS,
        "activation.double_tap_window_ms",
    );
    clamp_u64(
        &mut config.activation.cooldown_ms,
        BOUNDS_COOLDOWN_MS,
        DEFAULT_COOLDOWN_MS,
        "activation.cooldown_ms",
    );

    // Quote section.
    clamp_u32(
        &mut config.quote.max_display_lines,
        BOUNDS_QUOTE_MAX_DISPLAY_LINES,
        DEFAULT_QUOTE_MAX_DISPLAY_LINES,
        "quote.max_display_lines",
    );
    clamp_u32(
        &mut config.quote.max_display_chars,
        BOUNDS_QUOTE_MAX_DISPLAY_CHARS,
        DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
        "quote.max_display_chars",
    );
    clamp_u32(
        &mut config.quote.max_context_length,
        BOUNDS_QUOTE_MAX_CONTEXT_LENGTH,
        DEFAULT_QUOTE_MAX_CONTEXT_LENGTH,
        "quote.max_context_length",
    );
}

/// Composes the user-editable base prompt with the generated slash-command
/// appendix. The result is what `ask_ollama` actually sends to Ollama. The
/// file stores only the base; the appendix is never round-tripped.
pub fn compose_system_prompt(base: &str, appendix: &str) -> String {
    let base = base.trim_end();
    let appendix = appendix.trim();
    if appendix.is_empty() {
        base.to_string()
    } else {
        format!("{base}\n\n{appendix}")
    }
}

fn clamp_f64(value: &mut f64, bounds: (f64, f64), default: f64, field: &str) {
    if !value.is_finite() || !(bounds.0..=bounds.1).contains(value) {
        eprintln!(
            "thuki: [config] {field}={value} out of bounds [{min}, {max}]; using default {default}",
            min = bounds.0,
            max = bounds.1,
            value = *value
        );
        *value = default;
    }
}

fn clamp_u64(value: &mut u64, bounds: (u64, u64), default: u64, field: &str) {
    if !(bounds.0..=bounds.1).contains(value) {
        eprintln!(
            "thuki: [config] {field}={value} out of bounds [{min}, {max}]; using default {default}",
            min = bounds.0,
            max = bounds.1,
            value = *value
        );
        *value = default;
    }
}

fn clamp_u32(value: &mut u32, bounds: (u32, u32), default: u32, field: &str) {
    if !(bounds.0..=bounds.1).contains(value) {
        eprintln!(
            "thuki: [config] {field}={value} out of bounds [{min}, {max}]; using default {default}",
            min = bounds.0,
            max = bounds.1,
            value = *value
        );
        *value = default;
    }
}
