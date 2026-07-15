//! Typed shape of the Thuki configuration file.
//!
//! Serde derives the TOML mapping automatically. Each section struct carries
//! `#[serde(default)]` so a partial file (missing whole sections or fields)
//! deserializes cleanly: missing fields inherit the compiled defaults via the
//! manual `Default` impls below.
//!
//! Section structs use manual `Default` impls (NOT `#[derive(Default)]`)
//! because deriving Default would fill fields with zero/empty values
//! (`String::default() == ""`, `u64::default() == 0`), which is the opposite
//! of what the user expects. `AppConfig` itself uses `#[derive(Default)]`
//! because it delegates entirely to each section's own `Default` impl.

use serde::{Deserialize, Serialize};

use super::defaults::{
    DEFAULT_ACTIVE_PROVIDER, DEFAULT_AUTO_CLOSE, DEFAULT_AUTO_REPLACE, DEFAULT_AUTO_SEARCH,
    DEFAULT_BUILTIN_LABEL, DEFAULT_DEBUG_TRACE_ENABLED, DEFAULT_KEEP_WARM_INACTIVITY_MINUTES,
    DEFAULT_MAX_CHAT_HEIGHT, DEFAULT_MAX_IMAGES, DEFAULT_NUM_CTX, DEFAULT_OLLAMA_LABEL,
    DEFAULT_OLLAMA_URL, DEFAULT_OVERLAY_WIDTH, DEFAULT_QUOTE_MAX_CONTEXT_LENGTH,
    DEFAULT_QUOTE_MAX_DISPLAY_CHARS, DEFAULT_QUOTE_MAX_DISPLAY_LINES,
    DEFAULT_SEARCH_NOTICE_ACKNOWLEDGED, DEFAULT_SYSTEM_CUSTOMIZED, DEFAULT_SYSTEM_PROMPT_BASE,
    DEFAULT_TEXT_BASE_PX, DEFAULT_TEXT_FONT_WEIGHT, DEFAULT_TEXT_LETTER_SPACING_PX,
    DEFAULT_TEXT_LINE_HEIGHT, DEFAULT_TRACE_RETENTION_DAYS, DEFAULT_UPDATER_AUTO_CHECK,
    DEFAULT_UPDATER_CHECK_INTERVAL_HOURS, DEFAULT_UPDATER_MANIFEST_URL, PROVIDER_ID_BUILTIN,
    PROVIDER_ID_OLLAMA, PROVIDER_KIND_BUILTIN, PROVIDER_KIND_OLLAMA, PROVIDER_KIND_OPENAI,
};

/// A single configured inference provider. Exactly one is active at a time
/// (see [`InferenceSection::active_provider`]). The built-in entry is always
/// present and cannot be removed; the loader re-seeds it if a user file omits
/// it. Per-provider `model` replaces the former single SQLite `active_model`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Provider {
    /// Stable identifier referenced by `active_provider`.
    pub id: String,
    /// Provider kind: `"builtin"`, `"ollama"`, or `"openai"`. Unknown kinds
    /// are dropped by the loader.
    pub kind: String,
    /// Human-readable name shown in Settings.
    pub label: String,
    /// Base URL for network providers (Ollama, OpenAI-compatible). Empty for
    /// the built-in engine.
    pub base_url: String,
    /// The model selected for this provider. Empty means "none chosen yet".
    pub model: String,
    /// Manual vision flag for `openai`-kind providers. OpenAI-compatible local
    /// servers expose no capability probe, so the user declares whether the
    /// selected model accepts image inputs. Ignored for `builtin` and `ollama`,
    /// whose capabilities are resolved from the manifest or Ollama's
    /// `/api/show` response.
    #[serde(default)]
    pub vision: bool,
}

/// The built-in provider record (Thuki's own engine; no URL).
pub fn builtin_provider() -> Provider {
    Provider {
        id: PROVIDER_ID_BUILTIN.to_string(),
        kind: PROVIDER_KIND_BUILTIN.to_string(),
        label: DEFAULT_BUILTIN_LABEL.to_string(),
        base_url: String::new(),
        model: String::new(),
        vision: false,
    }
}

/// An Ollama provider record seeded with the given base URL.
pub fn ollama_provider(base_url: &str) -> Provider {
    Provider {
        id: PROVIDER_ID_OLLAMA.to_string(),
        kind: PROVIDER_KIND_OLLAMA.to_string(),
        label: DEFAULT_OLLAMA_LABEL.to_string(),
        base_url: base_url.to_string(),
        model: String::new(),
        vision: false,
    }
}

/// An OpenAI-compatible provider record with the given id, label, and base URL.
/// `vision` defaults to `false`; the caller or user sets it to `true` when the
/// selected model accepts image inputs.
pub fn openai_provider(id: &str, label: &str, base_url: &str) -> Provider {
    Provider {
        id: id.to_string(),
        kind: PROVIDER_KIND_OPENAI.to_string(),
        label: label.to_string(),
        base_url: base_url.to_string(),
        model: String::new(),
        vision: false,
    }
}

/// The default provider list: built-in first, then Ollama at localhost.
pub fn default_providers() -> Vec<Provider> {
    vec![builtin_provider(), ollama_provider(DEFAULT_OLLAMA_URL)]
}

/// Static, user-tunable inference configuration.
///
/// Inference targets one of several `providers`; `active_provider` selects
/// which. Per-provider model selection lives on each [`Provider`] record
/// (replacing the former single SQLite `active_model`). `num_ctx` and
/// `keep_warm_inactivity_minutes` are universal knobs read by both local
/// provider paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct InferenceSection {
    /// Id of the provider Thuki currently sends inference to. The loader
    /// repairs an empty or dangling pointer to `DEFAULT_ACTIVE_PROVIDER`.
    pub active_provider: String,
    /// Context window size (in tokens) sent to the active provider with every
    /// request. Warmup and chat use the same value so Ollama reuses the same
    /// runner instance and its cached KV prefix for the system prompt. Raise to
    /// fit longer conversations in a single context; lower to use less VRAM.
    /// Valid range: 2048..=1048576.
    pub num_ctx: u32,
    /// Minutes of inactivity before Thuki releases the active model from local
    /// memory. Governs both local providers: the built-in engine stops its
    /// sidecar, and Ollama is told to release the model. Not applicable to a
    /// remote OpenAI-compatible server, whose residency Thuki does not manage.
    /// 0 uses the provider's natural short default (~5 min): Ollama defers to
    /// its own timer, the built-in engine applies its own ~5-minute timer.
    /// -1 keeps the model resident indefinitely. Valid range: -1 or 0..=1440.
    pub keep_warm_inactivity_minutes: i32,
    /// The configured providers. Always contains the built-in entry after
    /// resolution. The field-level `#[serde(default)]` defaults a *missing*
    /// `providers` key to an empty Vec (not the seeded pair), so the loader can
    /// distinguish a pre-providers file (empty -> migrate from `ollama_url`)
    /// from a new-shape file with an explicit list. `resolve` always re-seeds
    /// the mandatory built-in and Ollama entries.
    #[serde(default)]
    pub providers: Vec<Provider>,
    /// Migration-only: the pre-providers `[inference] ollama_url` value. Read
    /// from old config files, consumed by `loader::resolve`, never written back.
    #[serde(default, rename = "ollama_url", skip_serializing)]
    pub legacy_ollama_url: Option<String>,
}

impl Default for InferenceSection {
    fn default() -> Self {
        Self {
            active_provider: DEFAULT_ACTIVE_PROVIDER.to_string(),
            num_ctx: DEFAULT_NUM_CTX,
            keep_warm_inactivity_minutes: DEFAULT_KEEP_WARM_INACTIVITY_MINUTES,
            providers: default_providers(),
            legacy_ollama_url: None,
        }
    }
}

impl InferenceSection {
    /// The active provider record, if `active_provider` resolves to one.
    pub fn active(&self) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == self.active_provider)
    }
    /// Base URL of the active provider (empty for the built-in / unresolved).
    pub fn active_provider_base_url(&self) -> &str {
        self.active().map(|p| p.base_url.as_str()).unwrap_or("")
    }
    /// The active provider's selected model (empty if none).
    pub fn active_provider_model(&self) -> &str {
        self.active().map(|p| p.model.as_str()).unwrap_or("")
    }
    /// The active provider's selected model as an `Option`, mapping an empty
    /// model field to `None` so callers can feed it straight into the
    /// active-model resolve helpers.
    pub fn active_provider_model_opt(&self) -> Option<&str> {
        let model = self.active_provider_model();
        (!model.is_empty()).then_some(model)
    }
    /// The active provider's kind (empty if unresolved).
    pub fn active_provider_kind(&self) -> &str {
        self.active().map(|p| p.kind.as_str()).unwrap_or("")
    }
}

/// Prompt configuration. `system` holds the user-editable persona prompt; on
/// first run it is seeded with the full built-in body so the file is the
/// single source of truth. The slash-command appendix is composed at load
/// time into `resolved_system` and is never written back to the file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PromptSection {
    /// User-editable persona prompt. Seeded with the built-in body and
    /// freely editable thereafter. If the user clears it (with
    /// `system_customized` set), no persona is sent (only the
    /// slash-command appendix).
    pub system: String,
    /// Set to `true` the first time the user explicitly saves the system
    /// prompt via Settings. While `false`, the persisted `system` is not
    /// authoritative (it is only a cached copy of the default seeded at
    /// first run), so the loader always refreshes it to the current
    /// `DEFAULT_SYSTEM_PROMPT_BASE`. Once `true`, the stored value is kept
    /// verbatim, including an explicit empty (which sends no persona). This
    /// both heals pre-Settings-UI configs (where `system = ""` was the old
    /// compiled default) and propagates later edits of the built-in prompt
    /// to every non-customizing install.
    pub system_customized: bool,
    /// Composed runtime value (base prompt plus slash-command appendix).
    /// Not serialized; computed by the loader.
    #[serde(skip)]
    pub resolved_system: String,
}

impl Default for PromptSection {
    fn default() -> Self {
        Self {
            system: DEFAULT_SYSTEM_PROMPT_BASE.to_string(),
            system_customized: DEFAULT_SYSTEM_CUSTOMIZED,
            resolved_system: String::new(),
        }
    }
}

/// Overlay UI configuration. Holds window geometry and input attachment
/// limits. The collapsed-bar height and the close-animation deadline are
/// baked into the frontend (see `App.tsx`) because their effective range is
/// invisible to the user (collapsed height is overwritten by the
/// ResizeObserver within a frame; the hide delay sits below normal perception
/// across its usable range and creates a visible pop if dropped below the
/// exit-animation duration).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WindowSection {
    /// Logical width of the overlay window.
    pub overlay_width: f64,
    /// Maximum height the expanded chat window is allowed to grow to.
    pub max_chat_height: f64,
    /// Maximum number of manually attached images per message. One additional
    /// image from /screen capture is allowed on top, for a total of
    /// max_images + 1 per message.
    pub max_images: u32,
    /// Base font size (in CSS pixels) for chat text and the AskBar input.
    /// Drives the `--thuki-text-base` CSS variable consumed by the AI
    /// markdown body, the user chat bubble text, and the AskBar textarea
    /// (plus its caret-tracking mirror). Other UI surfaces keep fixed sizes.
    /// Valid range: 11.0..=22.0.
    pub text_base_px: f64,
    /// Line-height multiplier applied to chat + AskBar text. Drives the
    /// `--thuki-text-line-height` CSS variable. Valid range: 1.0..=2.5.
    pub text_line_height: f64,
    /// Letter spacing (in CSS pixels) applied to chat + AskBar text.
    /// Drives the `--thuki-text-letter-spacing` CSS variable. Negative
    /// values tighten the typography; positive values airy it out.
    /// Valid range: -0.5..=2.0.
    pub text_letter_spacing_px: f64,
    /// CSS `font-weight` applied to chat + AskBar text. Drives the
    /// `--thuki-text-font-weight` CSS variable. Restricted to the four
    /// loaded Nunito weights (400, 500, 600, 700); values outside this
    /// set reset to the compiled default.
    pub text_font_weight: u32,
}

impl Default for WindowSection {
    fn default() -> Self {
        Self {
            overlay_width: DEFAULT_OVERLAY_WIDTH,
            max_chat_height: DEFAULT_MAX_CHAT_HEIGHT,
            max_images: DEFAULT_MAX_IMAGES,
            text_base_px: DEFAULT_TEXT_BASE_PX,
            text_line_height: DEFAULT_TEXT_LINE_HEIGHT,
            text_letter_spacing_px: DEFAULT_TEXT_LETTER_SPACING_PX,
            text_font_weight: DEFAULT_TEXT_FONT_WEIGHT,
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

/// Selection-replacement and web-search mode behavior.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct BehaviorSection {
    /// When `true`, a `/rewrite` or `/refine` result is written straight back
    /// into the source app (replacing the selection) as soon as the model
    /// finishes, with no Replace-button click required. When `false`, the
    /// user triggers the write manually via the in-chat Replace button.
    pub auto_replace: bool,
    /// When `true`, the overlay dismisses itself after a `/rewrite` or
    /// `/refine` result is successfully replaced into the source app, whether
    /// the replace was automatic (`auto_replace`) or a manual Replace click.
    /// Independent of `auto_replace`; only closes on a successful replace.
    pub auto_close: bool,
    /// When `true` (default), built-in auto-search may open the web on plain
    /// turns. When `false`, only `/search` forces a live web look-up.
    pub auto_search: bool,
    /// When `true`, the first-use web-search notice has been dismissed and
    /// should not show again. Default `false` until the user acknowledges it.
    pub search_notice_acknowledged: bool,
}

impl Default for BehaviorSection {
    fn default() -> Self {
        Self {
            auto_replace: DEFAULT_AUTO_REPLACE,
            auto_close: DEFAULT_AUTO_CLOSE,
            auto_search: DEFAULT_AUTO_SEARCH,
            search_notice_acknowledged: DEFAULT_SEARCH_NOTICE_ACKNOWLEDGED,
        }
    }
}

/// Developer and power-user debugging knobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DebugSection {
    /// Records every chat conversation (including the built-in web-search
    /// turns the `/search` command and the auto-search pre-pass drive) to
    /// JSON-Lines files under `app_data_dir/traces/chat/<conversation_id>.jsonl`.
    /// Off by default; toggleable from Settings.
    pub trace_enabled: bool,

    /// How many days recorded trace files are kept before they are pruned.
    /// Defaults to 7 days; the sentinel `-1` keeps them forever (never prune).
    /// Raise it to keep more history for later inspection, lower it to reclaim
    /// disk sooner. Enforced by a prune at startup and whenever the value
    /// changes from Settings.
    pub trace_retention_days: i64,
}

impl Default for DebugSection {
    fn default() -> Self {
        Self {
            trace_enabled: DEFAULT_DEBUG_TRACE_ENABLED,
            trace_retention_days: DEFAULT_TRACE_RETENTION_DAYS,
        }
    }
}

/// Auto-update configuration. Determines whether and how often Thuki polls
/// for new releases via the bundled tauri-plugin-updater.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct UpdaterSection {
    /// Poll for updates automatically at startup and every
    /// `check_interval_hours` hours while running.
    #[serde(default = "default_updater_auto_check")]
    pub auto_check: bool,

    /// Hours between automatic background checks. Bound to 1..168.
    #[serde(default = "default_updater_check_interval_hours")]
    pub check_interval_hours: u64,

    /// URL to fetch the update manifest from.
    #[serde(default = "default_updater_manifest_url")]
    pub manifest_url: String,
}

fn default_updater_auto_check() -> bool {
    DEFAULT_UPDATER_AUTO_CHECK
}
fn default_updater_check_interval_hours() -> u64 {
    DEFAULT_UPDATER_CHECK_INTERVAL_HOURS
}
fn default_updater_manifest_url() -> String {
    DEFAULT_UPDATER_MANIFEST_URL.to_string()
}

impl Default for UpdaterSection {
    fn default() -> Self {
        Self {
            auto_check: DEFAULT_UPDATER_AUTO_CHECK,
            check_interval_hours: DEFAULT_UPDATER_CHECK_INTERVAL_HOURS,
            manifest_url: DEFAULT_UPDATER_MANIFEST_URL.to_string(),
        }
    }
}

/// Top-level application configuration. Managed Tauri state; every subsystem
/// reads from `State<RwLock<AppConfig>>` and nowhere else. The loader resolves all
/// empty strings and out-of-bounds numerics to compiled defaults before the
/// `AppConfig` is installed, so every field here holds a usable value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct AppConfig {
    pub inference: InferenceSection,
    pub prompt: PromptSection,
    pub window: WindowSection,
    pub quote: QuoteSection,
    pub behavior: BehaviorSection,
    pub debug: DebugSection,
    #[serde(default)]
    pub updater: UpdaterSection,
}
