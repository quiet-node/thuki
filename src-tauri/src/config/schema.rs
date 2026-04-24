//! Typed shape of the Thuki configuration file.
//!
//! Serde derives the TOML mapping automatically. Each section struct carries
//! `#[serde(default)]` so a partial file (missing whole sections or fields)
//! deserializes cleanly: missing fields inherit the compiled defaults via the
//! manual `Default` impls below.
//!
//! Manual `Default` impls (NOT `#[derive(Default)]`) are used everywhere
//! because deriving Default would fill fields with zero/empty values
//! (`String::default() == ""`, `u64::default() == 0`), which is the opposite
//! of what the user expects.

use serde::{Deserialize, Serialize};

use super::defaults::{
    CURRENT_SCHEMA_VERSION, DEFAULT_COLLAPSED_HEIGHT, DEFAULT_COOLDOWN_MS,
    DEFAULT_DOUBLE_TAP_WINDOW_MS, DEFAULT_HIDE_COMMIT_DELAY_MS, DEFAULT_MAX_CHAT_HEIGHT,
    DEFAULT_MODEL_NAME, DEFAULT_OLLAMA_URL, DEFAULT_OVERLAY_WIDTH,
    DEFAULT_QUOTE_MAX_CONTEXT_LENGTH, DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
    DEFAULT_QUOTE_MAX_DISPLAY_LINES,
};

/// Model configuration. The first entry of `available` is the active model
/// used for all inference. Reorder the list (or use the future settings panel)
/// to switch models. Keeping a single list instead of separate `active` and
/// `available` fields eliminates the mismatch failure mode entirely.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModelSection {
    /// Ollama models Thuki knows about. First entry is active.
    pub available: Vec<String>,
    /// HTTP base URL of the local Ollama instance.
    pub ollama_url: String,
}

impl Default for ModelSection {
    fn default() -> Self {
        Self {
            available: vec![DEFAULT_MODEL_NAME.to_string()],
            ollama_url: DEFAULT_OLLAMA_URL.to_string(),
        }
    }
}

impl ModelSection {
    /// Returns the active model (first entry). Falls back to the compiled
    /// default if the list is somehow empty at call time; the loader also
    /// guarantees this never happens by calling `resolve` during load.
    pub fn active(&self) -> &str {
        self.available
            .first()
            .map(String::as_str)
            .unwrap_or(DEFAULT_MODEL_NAME)
    }
}

/// Prompt configuration. `system` holds only the user-editable base text.
/// The slash-command appendix is composed at load time into `resolved_system`
/// and is never written back to the file. `resolved_system` is computed, not
/// serialized.
///
/// Note: `#[derive(Default)]` is correct here because both fields genuinely
/// start empty: `system` empty means "use the built-in persona", and
/// `resolved_system` is populated by the loader before any consumer reads it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct PromptSection {
    /// User-editable persona prompt. Empty means "use the built-in default".
    pub system: String,
    /// Composed runtime value (base prompt plus slash-command appendix).
    /// Not serialized; computed by the loader.
    #[serde(skip)]
    pub resolved_system: String,
}

/// Overlay window geometry and animation timing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowSection {
    /// Logical width of the overlay window.
    pub overlay_width: f64,
    /// Height of the collapsed (AskBar) state.
    pub collapsed_height: f64,
    /// Maximum height the expanded chat window is allowed to grow to.
    pub max_chat_height: f64,
    /// Delay before actually hiding the NSPanel after the exit animation starts.
    pub hide_commit_delay_ms: u64,
}

impl Default for WindowSection {
    fn default() -> Self {
        Self {
            overlay_width: DEFAULT_OVERLAY_WIDTH,
            collapsed_height: DEFAULT_COLLAPSED_HEIGHT,
            max_chat_height: DEFAULT_MAX_CHAT_HEIGHT,
            hide_commit_delay_ms: DEFAULT_HIDE_COMMIT_DELAY_MS,
        }
    }
}

/// Hotkey activation timing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ActivationSection {
    /// Maximum gap between the two Control taps for them to count as a double tap.
    pub double_tap_window_ms: u64,
    /// Minimum wait between successful activations to avoid bounce.
    pub cooldown_ms: u64,
}

impl Default for ActivationSection {
    fn default() -> Self {
        Self {
            double_tap_window_ms: DEFAULT_DOUBLE_TAP_WINDOW_MS,
            cooldown_ms: DEFAULT_COOLDOWN_MS,
        }
    }
}

/// Selected-text quote display configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct QuoteSection {
    pub max_display_lines: u32,
    pub max_display_chars: u32,
    pub max_context_length: u32,
}

impl Default for QuoteSection {
    fn default() -> Self {
        Self {
            max_display_lines: DEFAULT_QUOTE_MAX_DISPLAY_LINES,
            max_display_chars: DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
            max_context_length: DEFAULT_QUOTE_MAX_CONTEXT_LENGTH,
        }
    }
}

/// Top-level application configuration. Managed Tauri state; every subsystem
/// reads from `State<AppConfig>` and nowhere else. The loader resolves all
/// empty strings and out-of-bounds numerics to compiled defaults before the
/// `AppConfig` is installed, so every field here holds a usable value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub model: ModelSection,
    pub prompt: PromptSection,
    pub window: WindowSection,
    pub activation: ActivationSection,
    pub quote: QuoteSection,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            model: ModelSection::default(),
            prompt: PromptSection::default(),
            window: WindowSection::default(),
            activation: ActivationSection::default(),
            quote: QuoteSection::default(),
        }
    }
}
