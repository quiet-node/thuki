//! Unit tests for the `config` module.
//!
//! These tests use temporary directories via `std::env::temp_dir()` with a
//! random UUID subdirectory (same pattern the rest of the codebase uses, see
//! `src-tauri/src/database.rs`). They cover every branch of `loader`, `writer`,
//! `schema`, and the helpers that feed them.
//!
//! The Tauri-aware `load(app)` wrapper at `config::load` is excluded from
//! coverage because it requires a real `AppHandle` and the real macOS
//! filesystem. Its internals delegate to `load_from_path`, which has full
//! coverage here.

use std::path::PathBuf;

use super::defaults::{
    DEFAULT_ACTIVE_PROVIDER, DEFAULT_AUTO_CLOSE, DEFAULT_AUTO_REPLACE, DEFAULT_AUTO_SEARCH,
    DEFAULT_DEBUG_TRACE_ENABLED, DEFAULT_JUDGE_TIMEOUT_S, DEFAULT_KEEP_WARM_INACTIVITY_MINUTES,
    DEFAULT_MAX_CHAT_HEIGHT, DEFAULT_MAX_IMAGES, DEFAULT_MAX_ITERATIONS, DEFAULT_NUM_CTX,
    DEFAULT_OLLAMA_URL, DEFAULT_OVERLAY_WIDTH, DEFAULT_QUOTE_MAX_CONTEXT_LENGTH,
    DEFAULT_QUOTE_MAX_DISPLAY_CHARS, DEFAULT_QUOTE_MAX_DISPLAY_LINES,
    DEFAULT_READER_BATCH_TIMEOUT_S, DEFAULT_READER_PER_URL_TIMEOUT_S, DEFAULT_READER_URL,
    DEFAULT_ROUTER_TIMEOUT_S, DEFAULT_SEARCH_TIMEOUT_S, DEFAULT_SEARXNG_MAX_RESULTS,
    DEFAULT_SEARXNG_URL, DEFAULT_SYSTEM_PROMPT_BASE, DEFAULT_TEXT_BASE_PX,
    DEFAULT_TEXT_FONT_WEIGHT, DEFAULT_TEXT_LETTER_SPACING_PX, DEFAULT_TEXT_LINE_HEIGHT,
    DEFAULT_TOP_K_URLS, DEFAULT_UPDATER_CHECK_INTERVAL_HOURS, DEFAULT_UPDATER_MANIFEST_URL,
    PROVIDER_ID_BUILTIN, PROVIDER_ID_OLLAMA, PROVIDER_KIND_BUILTIN, PROVIDER_KIND_OLLAMA,
    PROVIDER_KIND_OPENAI, SLASH_COMMAND_PROMPT_APPENDIX,
};
use super::error::ConfigError;
use super::loader::{compose_system_prompt, load_from_path, resolve};
use super::migrate::{attach_legacy_active_model, toml_has_providers};
use super::schema::{
    ollama_provider, openai_provider, AppConfig, BehaviorSection, DebugSection, InferenceSection,
    PromptSection, Provider, QuoteSection, SearchSection, UpdaterSection, WindowSection,
};
use super::writer::atomic_write;

/// Creates a fresh temp directory that is unique per test run. Returned paths
/// live inside `std::env::temp_dir()/thuki-config-tests-<uuid>/`.
fn fresh_temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("thuki-config-tests-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn config_path_in(dir: &std::path::Path) -> PathBuf {
    dir.join("config.toml")
}

/// Asserts `config` carries the compiled inference defaults: the built-in
/// provider is active and the seeded Ollama row keeps the default endpoint.
fn assert_default_inference(config: &AppConfig) {
    assert_eq!(config.inference.active_provider, DEFAULT_ACTIVE_PROVIDER);
    let ollama = config
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
        .expect("defaults seed an Ollama provider row");
    assert_eq!(ollama.base_url, DEFAULT_OLLAMA_URL);
}

// ── defaults module ──────────────────────────────────────────────────────────

#[test]
fn defaults_const_values_match_schema_defaults() {
    // Guard rail: a change to a default in defaults.rs must flow through to
    // AppConfig::default(). If this test fails, someone changed one but not both.
    let c = AppConfig::default();
    // Builtin is active by default and carries no base URL; the seeded
    // Ollama row still holds the compiled default endpoint.
    assert_eq!(c.inference.active_provider_base_url(), "");
    assert_default_inference(&c);
    assert_eq!(
        c.inference.keep_warm_inactivity_minutes,
        DEFAULT_KEEP_WARM_INACTIVITY_MINUTES
    );
    assert_eq!(c.inference.num_ctx, DEFAULT_NUM_CTX);
    assert_eq!(c.prompt.system, DEFAULT_SYSTEM_PROMPT_BASE);
    assert_eq!(c.prompt.resolved_system, "");
    assert_eq!(c.window.overlay_width, DEFAULT_OVERLAY_WIDTH);
    assert_eq!(c.window.max_chat_height, DEFAULT_MAX_CHAT_HEIGHT);
    assert_eq!(c.window.max_images, DEFAULT_MAX_IMAGES);
    assert_eq!(c.window.text_base_px, DEFAULT_TEXT_BASE_PX);
    assert_eq!(c.window.text_line_height, DEFAULT_TEXT_LINE_HEIGHT);
    assert_eq!(
        c.window.text_letter_spacing_px,
        DEFAULT_TEXT_LETTER_SPACING_PX
    );
    assert_eq!(c.window.text_font_weight, DEFAULT_TEXT_FONT_WEIGHT);
    assert_eq!(c.quote.max_display_lines, DEFAULT_QUOTE_MAX_DISPLAY_LINES);
    assert_eq!(c.quote.max_display_chars, DEFAULT_QUOTE_MAX_DISPLAY_CHARS);
    assert_eq!(c.quote.max_context_length, DEFAULT_QUOTE_MAX_CONTEXT_LENGTH);
    assert_eq!(c.search.searxng_url, DEFAULT_SEARXNG_URL);
    assert_eq!(c.search.reader_url, DEFAULT_READER_URL);
    assert_eq!(c.search.max_iterations, DEFAULT_MAX_ITERATIONS);
    assert_eq!(c.search.top_k_urls, DEFAULT_TOP_K_URLS);
    assert_eq!(c.search.searxng_max_results, DEFAULT_SEARXNG_MAX_RESULTS);
    assert_eq!(c.search.search_timeout_s, DEFAULT_SEARCH_TIMEOUT_S);
    assert_eq!(
        c.search.reader_per_url_timeout_s,
        DEFAULT_READER_PER_URL_TIMEOUT_S
    );
    assert_eq!(
        c.search.reader_batch_timeout_s,
        DEFAULT_READER_BATCH_TIMEOUT_S
    );
    assert_eq!(c.search.judge_timeout_s, DEFAULT_JUDGE_TIMEOUT_S);
    assert_eq!(c.search.router_timeout_s, DEFAULT_ROUTER_TIMEOUT_S);
}

#[test]
fn defaults_prompt_base_is_nonempty() {
    // Guard against accidentally shipping an empty persona prompt.
    assert!(!DEFAULT_SYSTEM_PROMPT_BASE.trim().is_empty());
}

#[test]
fn fresh_default_active_provider_is_builtin() {
    // Thuki ships the bundled engine, so a fresh install starts on the
    // built-in provider. Existing configs keep whatever active_provider they
    // persisted (see the legacy pin tests below).
    assert_eq!(DEFAULT_ACTIVE_PROVIDER, PROVIDER_ID_BUILTIN);
    assert_eq!(
        InferenceSection::default().active_provider,
        PROVIDER_ID_BUILTIN
    );
}

// ── schema module ───────────────────────────────────────────────────────────

#[test]
fn section_defaults_are_sensible() {
    let m = InferenceSection::default();
    assert_eq!(m.active_provider, DEFAULT_ACTIVE_PROVIDER);
    // The default active provider is the builtin engine, which has no URL.
    assert_eq!(m.active_provider_base_url(), "");

    let p = PromptSection::default();
    assert_eq!(p.system, DEFAULT_SYSTEM_PROMPT_BASE);

    let w = WindowSection::default();
    assert_eq!(w.overlay_width, DEFAULT_OVERLAY_WIDTH);
    assert_eq!(w.max_images, DEFAULT_MAX_IMAGES);
    assert_eq!(w.text_base_px, DEFAULT_TEXT_BASE_PX);
    assert_eq!(w.text_line_height, DEFAULT_TEXT_LINE_HEIGHT);
    assert_eq!(w.text_letter_spacing_px, DEFAULT_TEXT_LETTER_SPACING_PX);
    assert_eq!(w.text_font_weight, DEFAULT_TEXT_FONT_WEIGHT);

    let q = QuoteSection::default();
    assert_eq!(q.max_display_lines, DEFAULT_QUOTE_MAX_DISPLAY_LINES);
}

#[test]
fn app_config_serde_round_trip_matches_defaults() {
    let original = AppConfig::default();
    let toml_str = toml::to_string_pretty(&original).expect("serialize");
    let parsed: AppConfig = toml::from_str(&toml_str).expect("deserialize");
    // prompt.resolved_system is marked #[serde(skip)] so it does not round-trip
    // through the file. Compare everything else.
    assert_eq!(parsed.inference, original.inference);
    assert_eq!(parsed.prompt.system, original.prompt.system);
    assert_eq!(parsed.window, original.window);
    assert_eq!(parsed.quote, original.quote);
}

#[test]
fn app_config_partial_file_fills_missing_fields_with_defaults() {
    // Only declare one field; serde(default) fills the rest. A missing
    // `providers` key defaults to an empty Vec (field-level default), distinct
    // from the seeded pair, so the loader can detect a pre-providers file.
    let partial = r#"
        [inference]
        num_ctx = 32768
    "#;
    let parsed: AppConfig = toml::from_str(partial).expect("partial file parses");
    assert_eq!(parsed.inference.num_ctx, 32768);
    assert_eq!(parsed.inference.active_provider, DEFAULT_ACTIVE_PROVIDER);
    assert!(
        parsed.inference.providers.is_empty(),
        "a missing providers key deserializes to an empty Vec, not the seeded pair"
    );
    assert_eq!(parsed.window.overlay_width, DEFAULT_OVERLAY_WIDTH);
    assert_eq!(
        parsed.quote.max_display_lines,
        DEFAULT_QUOTE_MAX_DISPLAY_LINES
    );
}

// ── compose_system_prompt ────────────────────────────────────────────────────

#[test]
fn compose_system_prompt_joins_with_blank_line() {
    let got = compose_system_prompt("hello", "world");
    assert_eq!(got, "hello\n\nworld");
}

#[test]
fn compose_system_prompt_trims_trailing_base_whitespace() {
    let got = compose_system_prompt("hello  \n", "world");
    assert_eq!(got, "hello\n\nworld");
}

#[test]
fn compose_system_prompt_skips_appendix_when_empty() {
    let got = compose_system_prompt("hello", "   ");
    assert_eq!(got, "hello");
}

#[test]
fn compose_system_prompt_skips_appendix_when_totally_empty() {
    let got = compose_system_prompt("hello", "");
    assert_eq!(got, "hello");
}

#[test]
fn compose_system_prompt_returns_appendix_only_when_base_empty() {
    let got = compose_system_prompt("", "world");
    assert_eq!(got, "world");
}

#[test]
fn compose_system_prompt_returns_appendix_only_when_base_whitespace() {
    let got = compose_system_prompt("   \n\t", "world");
    assert_eq!(got, "world");
}

// ── loader: first run (file missing) ────────────────────────────────────────

#[test]
fn load_missing_file_seeds_defaults_and_returns_them() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    assert!(!path.exists());

    let config = load_from_path(&path).expect("seed on first run");

    assert!(path.exists(), "file should be seeded");
    assert_default_inference(&config);
    // Resolved system prompt composed from default base plus appendix.
    assert!(config
        .prompt
        .resolved_system
        .contains(DEFAULT_SYSTEM_PROMPT_BASE.trim()));
    assert!(config
        .prompt
        .resolved_system
        .contains(SLASH_COMMAND_PROMPT_APPENDIX.trim()));
}

#[test]
fn load_missing_file_in_missing_parent_dir_creates_dir() {
    let dir = fresh_temp_dir();
    let nested = dir.join("does").join("not").join("exist").join("yet");
    let path = config_path_in(&nested);
    let config = load_from_path(&path).expect("creates parent dir and seeds");
    assert!(path.exists());
    assert_default_inference(&config);
}

#[test]
fn load_seed_failure_returns_seed_failed() {
    // Parent is "/" on macOS: we have no permission to write config.toml there.
    // Using the literal path forces the writer to hit a PermissionDenied.
    let forbidden_path = PathBuf::from("/config.toml");
    match load_from_path(&forbidden_path) {
        Err(ConfigError::SeedFailed { path, .. }) => {
            assert_eq!(path, forbidden_path);
        }
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("expected SeedFailed but got Ok - is the test running as root?"),
    }
}

// ── loader: valid existing file ─────────────────────────────────────────────

#[test]
fn load_existing_valid_file_returns_resolved_config() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            ollama_url = "http://localhost:99999"
        "#,
    )
    .unwrap();

    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.inference.active_provider_base_url(),
        "http://localhost:99999"
    );
}

#[test]
fn load_existing_empty_file_returns_defaults() {
    // An empty file is valid TOML (no keys), so it parses and then the resolver
    // fills everything in.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config, expected_defaults_with_resolved_prompt());
}

fn expected_defaults_with_resolved_prompt() -> AppConfig {
    // Helper: the expected config after loading an empty / all-defaults file.
    // prompt.resolved_system is composed at load time.
    let mut c = AppConfig::default();
    c.prompt.resolved_system =
        compose_system_prompt(DEFAULT_SYSTEM_PROMPT_BASE, SLASH_COMMAND_PROMPT_APPENDIX);
    c
}

// ── loader: corrupt file ────────────────────────────────────────────────────

#[test]
fn load_corrupt_file_is_renamed_and_reseeded() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "this is = definitely not [ valid toml").unwrap();

    let config = load_from_path(&path).expect("recover from corrupt file");
    assert_default_inference(&config);

    // Original file renamed with .corrupt- prefix.
    let renamed_exists = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("config.toml.corrupt-")
        });
    assert!(renamed_exists, "corrupt file should be renamed");

    // Fresh defaults file exists at the original path.
    assert!(path.exists());
}

// ── loader: read error (not NotFound) ───────────────────────────────────────

#[cfg(unix)]
#[test]
fn load_unreadable_file_returns_in_memory_defaults() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[inference]\nollama_url = \"http://127.0.0.1:11434\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

    // If the current user is root, the permission bits are ignored and this
    // test is vacuous. Skip rather than falsely passing.
    if std::fs::read(&path).is_ok() {
        // Restore perms so the tempdir cleanup can succeed, then skip.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        return;
    }

    let config = load_from_path(&path).expect("fallback to in-memory defaults");
    assert_default_inference(&config);
    // Restore so cleanup works.
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
}

// ── loader: resolve (empties and bounds) ────────────────────────────────────

#[test]
fn resolve_unknown_model_field_is_ignored() {
    // Older config files seeded a `[inference] available = [...]` list. After
    // removing that field from the schema, serde must silently drop it
    // rather than refusing to parse the file.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            available = ["legacy:model", "another:model"]
            ollama_url = "http://localhost:11434"
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.inference.active_provider_base_url(),
        "http://localhost:11434"
    );
}

#[test]
fn resolve_keep_warm_inactivity_zero_is_valid() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            keep_warm_inactivity_minutes = 0
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.inference.keep_warm_inactivity_minutes, 0);
}

#[test]
fn resolve_keep_warm_inactivity_below_minus_one_falls_back_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            keep_warm_inactivity_minutes = -999
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.inference.keep_warm_inactivity_minutes,
        DEFAULT_KEEP_WARM_INACTIVITY_MINUTES
    );
}

#[test]
fn resolve_keep_warm_inactivity_valid_values_are_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            keep_warm_inactivity_minutes = 60
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.inference.keep_warm_inactivity_minutes, 60);
}

#[test]
fn resolve_keep_warm_inactivity_minus_one_is_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            keep_warm_inactivity_minutes = -1
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.inference.keep_warm_inactivity_minutes, -1);
}

#[test]
fn resolve_keep_warm_inactivity_above_max_falls_back_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            keep_warm_inactivity_minutes = 1441
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.inference.keep_warm_inactivity_minutes,
        DEFAULT_KEEP_WARM_INACTIVITY_MINUTES
    );
}

#[test]
fn inference_keep_warm_inactivity_roundtrips_through_toml() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            keep_warm_inactivity_minutes = 60
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.inference.keep_warm_inactivity_minutes, 60);

    atomic_write(&path, &config).unwrap();
    let reloaded = load_from_path(&path).unwrap();
    assert_eq!(
        reloaded.inference.keep_warm_inactivity_minutes,
        config.inference.keep_warm_inactivity_minutes,
    );
}

#[test]
fn resolve_num_ctx_default_matches_const() {
    let c = AppConfig::default();
    assert_eq!(c.inference.num_ctx, DEFAULT_NUM_CTX);
}

#[test]
fn resolve_num_ctx_below_lower_bound_falls_back_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[inference]\nnum_ctx = 1000\n").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.inference.num_ctx, DEFAULT_NUM_CTX);
}

#[test]
fn resolve_num_ctx_above_upper_bound_falls_back_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[inference]\nnum_ctx = 3000000\n").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.inference.num_ctx, DEFAULT_NUM_CTX);
}

#[test]
fn resolve_num_ctx_in_bounds_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[inference]\nnum_ctx = 32768\n").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.inference.num_ctx, 32768);
}

#[test]
fn num_ctx_roundtrips_through_toml() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[inference]\nnum_ctx = 32768\n").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.inference.num_ctx, 32768);

    atomic_write(&path, &config).unwrap();
    let reloaded = load_from_path(&path).unwrap();
    assert_eq!(reloaded.inference.num_ctx, config.inference.num_ctx);
}

#[test]
fn resolve_empty_ollama_url_falls_back() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            ollama_url = "   "
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.inference.active_provider_base_url(),
        DEFAULT_OLLAMA_URL
    );
}

#[test]
fn resolve_empty_system_prompt_without_customized_flag_uses_built_in_default() {
    // Upgrade migration path: old configs have system="" because that was the
    // compiled default before the Settings UI existed. Without system_customized,
    // the loader restores the built-in persona so upgraded users are not silently
    // left with no system prompt.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [prompt]
            system = ""
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.prompt.system, DEFAULT_SYSTEM_PROMPT_BASE);
    assert!(config
        .prompt
        .resolved_system
        .contains(DEFAULT_SYSTEM_PROMPT_BASE.trim()));
    assert!(config
        .prompt
        .resolved_system
        .contains(SLASH_COMMAND_PROMPT_APPENDIX.trim()));
}

#[test]
fn resolve_empty_system_prompt_with_customized_flag_keeps_only_appendix() {
    // Intentional clear: user opened Settings, cleared the prompt, and saved.
    // set_config_field co-writes system_customized=true so the loader respects
    // the deliberate empty and does not restore the built-in default.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [prompt]
            system = ""
            system_customized = true
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.prompt.system, "");
    assert_eq!(
        config.prompt.resolved_system,
        SLASH_COMMAND_PROMPT_APPENDIX.trim()
    );
    assert!(!config
        .prompt
        .resolved_system
        .contains(DEFAULT_SYSTEM_PROMPT_BASE.trim()));
}

#[test]
fn resolve_custom_system_prompt_flows_through_with_appendix() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [prompt]
            system = "You are a custom assistant."
            system_customized = true
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert!(config
        .prompt
        .resolved_system
        .contains("You are a custom assistant."));
    assert!(config
        .prompt
        .resolved_system
        .contains(SLASH_COMMAND_PROMPT_APPENDIX.trim()));
}

#[test]
fn resolve_stale_noncustomized_system_prompt_is_refreshed_to_default() {
    // Migration path: a config seeded with an older build's prompt holds a
    // non-empty `system` with system_customized=false. The persisted text is
    // not authoritative, so resolve replaces it with the current compiled
    // default rather than letting the stale prompt flow through.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [prompt]
            system = "An older default prompt that should be refreshed."
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.prompt.system, DEFAULT_SYSTEM_PROMPT_BASE);
    assert!(!config
        .prompt
        .resolved_system
        .contains("An older default prompt that should be refreshed."));
    assert!(config
        .prompt
        .resolved_system
        .contains(DEFAULT_SYSTEM_PROMPT_BASE.trim()));
}

#[test]
fn resolve_out_of_bounds_floats_reset_to_defaults() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [window]
            overlay_width = 0.0
            max_chat_height = -1.0
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.overlay_width, DEFAULT_OVERLAY_WIDTH);
    assert_eq!(config.window.max_chat_height, DEFAULT_MAX_CHAT_HEIGHT);
}

#[test]
fn resolve_out_of_bounds_max_images_resets_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[window]\nmax_images = 0\n").unwrap();
    let config_low = load_from_path(&path).unwrap();
    assert_eq!(config_low.window.max_images, DEFAULT_MAX_IMAGES);

    std::fs::write(&path, "[window]\nmax_images = 99\n").unwrap();
    let config_high = load_from_path(&path).unwrap();
    assert_eq!(config_high.window.max_images, DEFAULT_MAX_IMAGES);
}

#[test]
fn resolve_out_of_bounds_text_base_px_resets_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[window]\ntext_base_px = 6.0\n").unwrap();
    let too_low = load_from_path(&path).unwrap();
    assert_eq!(too_low.window.text_base_px, DEFAULT_TEXT_BASE_PX);

    std::fs::write(&path, "[window]\ntext_base_px = 99.0\n").unwrap();
    let too_high = load_from_path(&path).unwrap();
    assert_eq!(too_high.window.text_base_px, DEFAULT_TEXT_BASE_PX);

    std::fs::write(&path, "[window]\ntext_base_px = nan\n").unwrap();
    let non_finite = load_from_path(&path).unwrap();
    assert_eq!(non_finite.window.text_base_px, DEFAULT_TEXT_BASE_PX);
}

#[test]
fn resolve_text_base_px_in_bounds_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[window]\ntext_base_px = 18.0\n").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.text_base_px, 18.0);
}

#[test]
fn resolve_out_of_bounds_text_line_height_resets_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[window]\ntext_line_height = 0.5\n").unwrap();
    let too_low = load_from_path(&path).unwrap();
    assert_eq!(too_low.window.text_line_height, DEFAULT_TEXT_LINE_HEIGHT);

    std::fs::write(&path, "[window]\ntext_line_height = 9.0\n").unwrap();
    let too_high = load_from_path(&path).unwrap();
    assert_eq!(too_high.window.text_line_height, DEFAULT_TEXT_LINE_HEIGHT);

    std::fs::write(&path, "[window]\ntext_line_height = nan\n").unwrap();
    let non_finite = load_from_path(&path).unwrap();
    assert_eq!(non_finite.window.text_line_height, DEFAULT_TEXT_LINE_HEIGHT);
}

#[test]
fn resolve_out_of_bounds_text_letter_spacing_resets_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[window]\ntext_letter_spacing_px = -5.0\n").unwrap();
    let too_low = load_from_path(&path).unwrap();
    assert_eq!(
        too_low.window.text_letter_spacing_px,
        DEFAULT_TEXT_LETTER_SPACING_PX
    );

    std::fs::write(&path, "[window]\ntext_letter_spacing_px = 10.0\n").unwrap();
    let too_high = load_from_path(&path).unwrap();
    assert_eq!(
        too_high.window.text_letter_spacing_px,
        DEFAULT_TEXT_LETTER_SPACING_PX
    );

    std::fs::write(&path, "[window]\ntext_letter_spacing_px = nan\n").unwrap();
    let non_finite = load_from_path(&path).unwrap();
    assert_eq!(
        non_finite.window.text_letter_spacing_px,
        DEFAULT_TEXT_LETTER_SPACING_PX
    );
}

#[test]
fn resolve_invalid_text_font_weight_resets_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[window]\ntext_font_weight = 123\n").unwrap();
    let off_grid = load_from_path(&path).unwrap();
    assert_eq!(off_grid.window.text_font_weight, DEFAULT_TEXT_FONT_WEIGHT);

    std::fs::write(&path, "[window]\ntext_font_weight = 800\n").unwrap();
    let above_set = load_from_path(&path).unwrap();
    assert_eq!(above_set.window.text_font_weight, DEFAULT_TEXT_FONT_WEIGHT);
}

#[test]
fn resolve_text_typography_in_bounds_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[window]\ntext_line_height = 1.8\ntext_letter_spacing_px = 0.4\ntext_font_weight = 700\n",
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.text_line_height, 1.8);
    assert_eq!(config.window.text_letter_spacing_px, 0.4);
    assert_eq!(config.window.text_font_weight, 700);
}

#[test]
fn resolve_max_images_in_bounds_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[window]\nmax_images = 5\n").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.max_images, 5);
}

#[test]
fn resolve_non_finite_float_resets() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [window]
            overlay_width = nan
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.overlay_width, DEFAULT_OVERLAY_WIDTH);
}

#[test]
fn resolve_out_of_bounds_u32_resets() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [quote]
            max_display_lines = 0
            max_display_chars = 99999
            max_context_length = 0
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.quote.max_display_lines,
        DEFAULT_QUOTE_MAX_DISPLAY_LINES
    );
    assert_eq!(
        config.quote.max_display_chars,
        DEFAULT_QUOTE_MAX_DISPLAY_CHARS
    );
    assert_eq!(
        config.quote.max_context_length,
        DEFAULT_QUOTE_MAX_CONTEXT_LENGTH
    );
}

#[test]
fn resolve_values_within_bounds_are_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [window]
            overlay_width = 800.0
            max_chat_height = 1000.0
            max_images = 7
            [quote]
            max_display_lines = 6
            max_display_chars = 500
            max_context_length = 8192
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.overlay_width, 800.0);
    assert_eq!(config.window.max_chat_height, 1000.0);
    assert_eq!(config.window.max_images, 7);
    assert_eq!(config.quote.max_display_lines, 6);
    assert_eq!(config.quote.max_display_chars, 500);
    assert_eq!(config.quote.max_context_length, 8192);
}

// ── rename_corrupt failure path ─────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn rename_corrupt_failure_is_logged_but_does_not_block() {
    use std::os::unix::fs::PermissionsExt;

    // Make the parent dir read-only so rename fails but we can still write the
    // fresh seed (wait - seed uses the same dir, so seeding would ALSO fail).
    // Instead use a different failure vector: make the source file immutable
    // by removing it after writing. Simulating this reliably in a unit test is
    // hard; the easiest reproducible path is a read-only directory where the
    // rename call fails but the test still exercises the code.
    //
    // Strategy: write a corrupt file to dir A. Make dir A read-only. Loader
    // will try rename within dir A (fails), then try to write a fresh file to
    // dir A (also fails), ending in SeedFailed. That exercises the "rename
    // failed, log, continue" arm AND the SeedFailed arm together.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "garbage").unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();

    let result = load_from_path(&path);
    // Restore so cleanup works.
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));

    // On root or permissive filesystems this may have succeeded; treat as vacuous.
    match result {
        Err(ConfigError::SeedFailed { .. }) => { /* expected */ }
        Ok(_) => {
            // Vacuous: test environment allowed both rename and reseed.
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }
}

/// Triggers ONLY the marker-write failure branch in `rename_corrupt`: the
/// rename itself must succeed (so the parent must be writable), but writing
/// the marker file must fail. We force the latter by pre-creating a
/// DIRECTORY at the marker path, which makes `std::fs::write` to that name
/// fail with `IsADirectory`. The corrupt file is still renamed and the
/// loader still seeds defaults; only the warning eprintln fires.
#[cfg(unix)]
#[test]
fn marker_write_failure_is_logged_but_does_not_block_recovery() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "garbage [oops").unwrap();

    // Squat the marker filename with a directory so std::fs::write fails.
    let blocker = dir.join(crate::config::CORRUPT_MARKER_FILE_NAME);
    std::fs::create_dir(&blocker).unwrap();

    let config = load_from_path(&path).expect("recover even when marker write fails");
    assert_default_inference(&config);

    // Marker squatter is still a directory: the failed write did not replace it.
    assert!(blocker.is_dir());
}

// ── writer: atomic_write ────────────────────────────────────────────────────

#[test]
fn atomic_write_creates_file_with_defaults() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    let mut config = AppConfig::default();
    config.prompt.resolved_system = "ignored".to_string();

    atomic_write(&path, &config).expect("write succeeds");
    let contents = std::fs::read_to_string(&path).unwrap();
    // resolved_system is not serialized (marked #[serde(skip)]).
    assert!(!contents.contains("resolved_system"));
    assert!(contents.contains("active_provider"));
}

#[cfg(unix)]
#[test]
fn atomic_write_file_has_mode_0600() {
    use std::os::unix::fs::PermissionsExt;
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    atomic_write(&path, &AppConfig::default()).unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "expected 0600, got {:o}", mode);
}

#[test]
fn atomic_write_creates_missing_parent_dir() {
    let dir = fresh_temp_dir();
    let nested = dir.join("a").join("b");
    let path = nested.join("config.toml");
    atomic_write(&path, &AppConfig::default()).unwrap();
    assert!(path.exists());
}

#[test]
fn atomic_write_overwrites_existing_file_atomically() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "old contents").unwrap();
    atomic_write(&path, &AppConfig::default()).unwrap();
    let contents = std::fs::read_to_string(&path).unwrap();
    assert!(contents.contains("active_provider"));
    assert!(!contents.contains("old contents"));
}

#[test]
fn atomic_write_fails_with_no_parent() {
    // Path literally "/" has no parent. On Unix this returns None from
    // Path::parent(), hitting the InvalidInput branch.
    let path = PathBuf::from("/");
    let err = atomic_write(&path, &AppConfig::default()).expect_err("root path has no parent");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[cfg(unix)]
#[test]
fn atomic_write_fails_when_parent_not_writable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = fresh_temp_dir();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
    let path = config_path_in(&dir);

    let result = atomic_write(&path, &AppConfig::default());
    // Restore so cleanup works.
    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));

    // Vacuous if running as root; otherwise we expect a permission error from
    // one of the write stages (create_dir_all, open, or rename).
    if let Err(err) = result {
        let kind = err.kind();
        assert!(
            matches!(
                kind,
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::Other
            ),
            "unexpected io error kind: {kind:?}"
        );
    }
}

/// create_dir_all fails when the parent path already exists as a regular file
/// (not a directory). This covers the `?` on create_dir_all in atomic_write.
#[test]
fn atomic_write_fails_when_parent_is_a_file() {
    let dir = fresh_temp_dir();
    let blocker = dir.join("blocker-file");
    std::fs::write(&blocker, "im a file not a dir").unwrap();
    let path = blocker.join("config.toml");
    let err = atomic_write(&path, &AppConfig::default()).expect_err("cannot mkdir over a file");
    // Depending on the platform this is NotADirectory or AlreadyExists; either
    // is a legitimate reason to bail. Just assert we got an error.
    let _ = err.kind();
}

/// Rename fails when the destination is an existing non-empty directory.
/// This covers the `?` on std::fs::rename in atomic_write.
#[test]
fn atomic_write_fails_when_destination_is_non_empty_directory() {
    let dir = fresh_temp_dir();
    let target = config_path_in(&dir);
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("filler"), "x").unwrap();
    let err = atomic_write(&target, &AppConfig::default())
        .expect_err("cannot rename over a non-empty directory");
    let _ = err.kind();
}

/// Failed rename removes the staged tmpfile so orphans do not accumulate in
/// the app-support directory across repeated write retries.
#[test]
fn atomic_write_cleans_up_tmpfile_on_rename_failure() {
    let dir = fresh_temp_dir();
    let target = config_path_in(&dir);
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("filler"), "x").unwrap();
    atomic_write(&target, &AppConfig::default()).expect_err("rename over non-empty dir must fail");
    let leftover_tmp = std::fs::read_dir(target.parent().unwrap())
        .unwrap()
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains(".tmp-"));
    assert!(
        !leftover_tmp,
        "atomic_write must remove its tmpfile on rename failure"
    );
}

// ── error display ────────────────────────────────────────────────────────────

#[test]
fn config_error_messages_include_context() {
    // Sanity-check the Display derives so log output stays useful.
    let e = ConfigError::SeedFailed {
        path: PathBuf::from("/tmp/x"),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
    };
    let msg = e.to_string();
    assert!(msg.contains("/tmp/x"));
    assert!(msg.contains("denied"));

    let e = ConfigError::IoError {
        path: PathBuf::from("/tmp/z"),
        source: std::io::Error::other("nope"),
    };
    assert!(e.to_string().contains("/tmp/z"));
}

// ── search section ────────────────────────────────────────────────────────────

#[test]
fn search_section_defaults_are_sane() {
    let s = SearchSection::default();
    assert!(s.searxng_url.starts_with("http://127.0.0.1:"));
    assert!(s.reader_url.starts_with("http://127.0.0.1:"));
    assert!(s.max_iterations >= 1 && s.max_iterations <= 10);
    assert!(s.top_k_urls >= 1 && s.top_k_urls <= 20);
    assert!(s.searxng_max_results >= 1 && s.searxng_max_results <= 20);
    assert!(s.reader_batch_timeout_s > s.reader_per_url_timeout_s);
}

#[test]
fn search_section_roundtrips_through_toml() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    let original = AppConfig::default();
    atomic_write(&path, &original).unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(loaded.search.searxng_url, original.search.searxng_url);
    assert_eq!(loaded.search.reader_url, original.search.reader_url);
    assert_eq!(loaded.search.max_iterations, original.search.max_iterations);
    assert_eq!(loaded.search.top_k_urls, original.search.top_k_urls);
    assert_eq!(
        loaded.search.searxng_max_results,
        original.search.searxng_max_results
    );
    assert_eq!(
        loaded.search.search_timeout_s,
        original.search.search_timeout_s
    );
    assert_eq!(
        loaded.search.reader_per_url_timeout_s,
        original.search.reader_per_url_timeout_s
    );
    assert_eq!(
        loaded.search.reader_batch_timeout_s,
        original.search.reader_batch_timeout_s
    );
    assert_eq!(
        loaded.search.judge_timeout_s,
        original.search.judge_timeout_s
    );
    assert_eq!(
        loaded.search.router_timeout_s,
        original.search.router_timeout_s
    );
}

#[test]
fn search_empty_url_resets_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[search]\nsearxng_url = \"\"\nreader_url = \"  \"\n").unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(loaded.search.searxng_url, DEFAULT_SEARXNG_URL);
    assert_eq!(loaded.search.reader_url, DEFAULT_READER_URL);
}

#[test]
fn search_max_iterations_clamped_to_bounds() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[search]\nmax_iterations = 0\ntop_k_urls = 999\n").unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(loaded.search.max_iterations, DEFAULT_MAX_ITERATIONS);
    assert_eq!(loaded.search.top_k_urls, DEFAULT_TOP_K_URLS);
}

#[test]
fn search_searxng_max_results_clamped_to_bounds() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    // Below lower bound (0) and above upper bound (999) both reset to default.
    std::fs::write(&path, "[search]\nsearxng_max_results = 0\n").unwrap();
    let loaded_low = load_from_path(&path).unwrap();
    assert_eq!(
        loaded_low.search.searxng_max_results,
        DEFAULT_SEARXNG_MAX_RESULTS
    );
    std::fs::write(&path, "[search]\nsearxng_max_results = 999\n").unwrap();
    let loaded_high = load_from_path(&path).unwrap();
    assert_eq!(
        loaded_high.search.searxng_max_results,
        DEFAULT_SEARXNG_MAX_RESULTS
    );
}

#[test]
fn search_searxng_max_results_in_bounds_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[search]\nsearxng_max_results = 5\n").unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(loaded.search.searxng_max_results, 5);
}

#[test]
fn search_timeouts_clamped_to_bounds() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[search]\nsearch_timeout_s = 0\nrouter_timeout_s = 9999\n",
    )
    .unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(loaded.search.search_timeout_s, DEFAULT_SEARCH_TIMEOUT_S);
    assert_eq!(loaded.search.router_timeout_s, DEFAULT_ROUTER_TIMEOUT_S);
}

#[test]
fn search_batch_timeout_invariant_corrected() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    // Set batch <= per_url — loader must correct.
    std::fs::write(
        &path,
        "[search]\nreader_per_url_timeout_s = 20\nreader_batch_timeout_s = 5\n",
    )
    .unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert!(
        loaded.search.reader_batch_timeout_s > loaded.search.reader_per_url_timeout_s,
        "loader must correct batch_timeout > per_url_timeout invariant"
    );
}

#[test]
fn toml_without_search_section_deserializes_to_defaults() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[inference]\nollama_url = \"http://127.0.0.1:11434\"\n",
    )
    .unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(
        loaded.search.searxng_url, DEFAULT_SEARXNG_URL,
        "missing [search] section must deserialize to defaults via #[serde(default)]"
    );
}

#[test]
fn toml_partial_search_section_fills_missing_fields_from_defaults() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[search]\nsearxng_url = \"http://192.168.1.50:8080\"\n",
    )
    .unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(loaded.search.searxng_url, "http://192.168.1.50:8080");
    assert_eq!(
        loaded.search.reader_url, DEFAULT_READER_URL,
        "unset field in partial [search] must fall back to default"
    );
    assert_eq!(loaded.search.max_iterations, DEFAULT_MAX_ITERATIONS);
}

// ── error: serde_json round-trip ────────────────────────────────────────────

/// `ConfigError::IoError` carries a non-Serialize `std::io::Error`; the
/// `serialize_io_error` helper is wired in via `#[serde(serialize_with)]`.
/// This round-trip test exists solely to exercise that helper so it shows
/// up as covered.
#[test]
fn config_error_io_error_serializes_io_source_as_display_string() {
    let err = ConfigError::IoError {
        path: PathBuf::from("/tmp/nope.toml"),
        source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied here"),
    };
    let json = serde_json::to_value(&err).expect("ConfigError serializes");
    assert_eq!(json["kind"], "io_error");
    assert_eq!(json["path"], "/tmp/nope.toml");
    assert_eq!(json["source"], "denied here");
}

// ── behavior section ────────────────────────────────────────────────────────

#[test]
fn behavior_section_default_matches_compiled_defaults() {
    let b = BehaviorSection::default();
    assert_eq!(b.auto_replace, DEFAULT_AUTO_REPLACE);
    assert_eq!(b.auto_close, DEFAULT_AUTO_CLOSE);
    assert_eq!(b.auto_search, DEFAULT_AUTO_SEARCH);
}

#[test]
fn app_config_default_includes_behavior_section_with_compiled_defaults() {
    let c = AppConfig::default();
    assert_eq!(c.behavior.auto_replace, DEFAULT_AUTO_REPLACE);
    assert_eq!(c.behavior.auto_close, DEFAULT_AUTO_CLOSE);
    assert_eq!(c.behavior.auto_search, DEFAULT_AUTO_SEARCH);
}

#[test]
fn behavior_auto_replace_round_trips_through_load() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[behavior]\nauto_replace = true\n").unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert!(loaded.behavior.auto_replace);
}

#[test]
fn behavior_auto_close_round_trips_through_load() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[behavior]\nauto_close = true\n").unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert!(loaded.behavior.auto_close);
}

#[test]
fn behavior_auto_search_round_trips_through_load() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[behavior]\nauto_search = false\n").unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert!(!loaded.behavior.auto_search);
}

#[test]
fn toml_without_behavior_section_deserializes_to_defaults() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[inference]\nollama_url = \"http://127.0.0.1:11434\"\n",
    )
    .unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(
        loaded.behavior.auto_replace, DEFAULT_AUTO_REPLACE,
        "missing [behavior] section must deserialize to defaults via #[serde(default)]"
    );
    assert_eq!(
        loaded.behavior.auto_close, DEFAULT_AUTO_CLOSE,
        "missing [behavior] section must deserialize to defaults via #[serde(default)]"
    );
    assert_eq!(
        loaded.behavior.auto_search, DEFAULT_AUTO_SEARCH,
        "missing [behavior] section must deserialize to defaults via #[serde(default)]"
    );
}

// ── debug section ───────────────────────────────────────────────────────────

#[test]
fn debug_section_default_matches_compiled_defaults() {
    let d = DebugSection::default();
    assert_eq!(d.trace_enabled, DEFAULT_DEBUG_TRACE_ENABLED);
}

#[test]
fn app_config_default_includes_debug_section_with_compiled_defaults() {
    let c = AppConfig::default();
    assert_eq!(c.debug.trace_enabled, DEFAULT_DEBUG_TRACE_ENABLED);
}

#[test]
fn debug_trace_enabled_round_trips_through_load() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[debug]\ntrace_enabled = true\n").unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert!(loaded.debug.trace_enabled);
}

#[test]
fn toml_without_debug_section_deserializes_to_defaults() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[inference]\nollama_url = \"http://127.0.0.1:11434\"\n",
    )
    .unwrap();
    let loaded = load_from_path(&path).unwrap();
    assert_eq!(
        loaded.debug.trace_enabled, DEFAULT_DEBUG_TRACE_ENABLED,
        "missing [debug] section must deserialize to defaults via #[serde(default)]"
    );
}

// ── updater section ──────────────────────────────────────────────────────────

#[test]
fn default_updater_section_matches_constants() {
    let s = UpdaterSection::default();
    assert!(s.auto_check);
    assert_eq!(s.check_interval_hours, DEFAULT_UPDATER_CHECK_INTERVAL_HOURS);
    assert_eq!(s.manifest_url, DEFAULT_UPDATER_MANIFEST_URL);
}

#[test]
fn updater_interval_too_small_resets_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[updater]\ncheck_interval_hours = 0\n").unwrap();
    let cfg = load_from_path(&path).unwrap();
    assert_eq!(
        cfg.updater.check_interval_hours,
        DEFAULT_UPDATER_CHECK_INTERVAL_HOURS
    );
}

#[test]
fn updater_interval_too_large_resets_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[updater]\ncheck_interval_hours = 999\n").unwrap();
    let cfg = load_from_path(&path).unwrap();
    assert_eq!(
        cfg.updater.check_interval_hours,
        DEFAULT_UPDATER_CHECK_INTERVAL_HOURS
    );
}

#[test]
fn updater_interval_in_bounds_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[updater]\ncheck_interval_hours = 6\n").unwrap();
    let cfg = load_from_path(&path).unwrap();
    assert_eq!(cfg.updater.check_interval_hours, 6);
}

#[test]
fn updater_empty_manifest_url_falls_back_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[updater]\nmanifest_url = \"   \"\n").unwrap();
    let cfg = load_from_path(&path).unwrap();
    assert_eq!(cfg.updater.manifest_url, DEFAULT_UPDATER_MANIFEST_URL);
}

#[test]
fn updater_toml_roundtrip_preserves_fields() {
    let original = AppConfig {
        updater: UpdaterSection {
            auto_check: false,
            check_interval_hours: 12,
            manifest_url: "https://example.com/m.json".to_string(),
        },
        ..AppConfig::default()
    };
    let serialized = toml::to_string(&original).unwrap();
    let roundtripped: AppConfig = toml::from_str(&serialized).unwrap();
    assert_eq!(roundtripped.updater, original.updater);
}

// ── inference providers: schema defaults ─────────────────────────────────────

#[test]
fn inference_defaults_seed_builtin_and_ollama_providers() {
    let c = AppConfig::default();
    assert_eq!(c.inference.active_provider, DEFAULT_ACTIVE_PROVIDER);
    assert_eq!(c.inference.active_provider_kind(), PROVIDER_KIND_BUILTIN);
    assert_eq!(c.inference.num_ctx, DEFAULT_NUM_CTX);
    assert_eq!(
        c.inference.keep_warm_inactivity_minutes,
        DEFAULT_KEEP_WARM_INACTIVITY_MINUTES
    );
    let ids: Vec<&str> = c
        .inference
        .providers
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(ids, vec![PROVIDER_ID_BUILTIN, PROVIDER_ID_OLLAMA]);
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
        .unwrap();
    assert_eq!(ollama.base_url, DEFAULT_OLLAMA_URL);
    assert_eq!(ollama.model, "");
    let builtin = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_BUILTIN)
        .unwrap();
    assert_eq!(builtin.base_url, "");
    assert_eq!(c.inference.legacy_ollama_url, None);
}

#[test]
fn provider_constructors_carry_expected_fields() {
    let b = super::schema::builtin_provider();
    assert_eq!(b.id, PROVIDER_ID_BUILTIN);
    assert_eq!(b.kind, PROVIDER_KIND_BUILTIN);
    assert!(b.base_url.is_empty());
    assert!(!b.vision);

    let o = super::schema::ollama_provider("http://x:1");
    assert_eq!(o.id, PROVIDER_ID_OLLAMA);
    assert_eq!(o.kind, PROVIDER_KIND_OLLAMA);
    assert_eq!(o.base_url, "http://x:1");
    assert!(!o.vision);
}

#[test]
fn active_provider_accessors_handle_missing_active() {
    // An InferenceSection whose active pointer matches no provider returns
    // empty strings rather than panicking.
    let inf = InferenceSection {
        active_provider: "ghost".to_string(),
        providers: vec![],
        ..InferenceSection::default()
    };
    assert!(inf.active().is_none());
    assert_eq!(inf.active_provider_base_url(), "");
    assert_eq!(inf.active_provider_model(), "");
    assert_eq!(inf.active_provider_model_opt(), None);
    assert_eq!(inf.active_provider_kind(), "");
}

#[test]
fn active_provider_model_opt_maps_empty_to_none() {
    // Empty model field -> None; a selected model -> Some(slug). Drives the
    // active-model resolve helpers without re-deriving the empty check.
    let mut c = AppConfig::default(); // active = builtin, model empty
    assert_eq!(c.inference.active_provider_model_opt(), None);
    if let Some(builtin) = c
        .inference
        .providers
        .iter_mut()
        .find(|p| p.id == PROVIDER_ID_BUILTIN)
    {
        builtin.model = "org/gemma:gemma.gguf".to_string();
    }
    assert_eq!(
        c.inference.active_provider_model_opt(),
        Some("org/gemma:gemma.gguf")
    );
}

// ── inference providers: migration matrix ────────────────────────────────────

#[test]
fn migrates_old_ollama_url_to_ollama_provider_and_activates_it() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[inference]\nollama_url = \"http://192.168.1.50:11434\"\n",
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert_eq!(c.inference.active_provider, PROVIDER_ID_OLLAMA);
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
        .unwrap();
    assert_eq!(ollama.base_url, "http://192.168.1.50:11434");
    assert!(c
        .inference
        .providers
        .iter()
        .any(|p| p.id == PROVIDER_ID_BUILTIN));
    // legacy field is consumed by resolve and never re-serialized.
    assert_eq!(c.inference.legacy_ollama_url, None);
}

#[test]
fn migrates_old_empty_ollama_url_to_localhost_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[inference]\nollama_url = \"\"\n").unwrap();
    let c = load_from_path(&path).unwrap();
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
        .unwrap();
    assert_eq!(ollama.base_url, DEFAULT_OLLAMA_URL);
}

#[test]
fn legacy_ollama_url_ignored_when_explicit_providers_present() {
    // Defensive: a hand-edited file carrying BOTH the legacy `ollama_url` and
    // an explicit providers list keeps the explicit providers; the legacy
    // value is consumed and dropped rather than overwriting them.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            ollama_url = "http://legacy-ignored:1"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://explicit:2"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
        .unwrap();
    assert_eq!(ollama.base_url, "http://explicit:2");
    assert_eq!(c.inference.legacy_ollama_url, None);
    // The migration branch must be short-circuited entirely: no duplicate
    // Ollama provider is synthesized alongside the explicit one.
    assert_eq!(
        c.inference
            .providers
            .iter()
            .filter(|p| p.id == PROVIDER_ID_OLLAMA)
            .count(),
        1
    );
}

#[test]
fn dangling_active_provider_falls_back_to_default() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "nonexistent"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert_eq!(c.inference.active_provider, DEFAULT_ACTIVE_PROVIDER);
}

#[test]
fn builtin_label_is_healed_to_the_current_default() {
    // A config seeded before the label was renamed keeps a stale built-in label
    // on disk. The loader must overwrite it with the current default so existing
    // installs do not keep showing an outdated provider name.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "builtin"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in (Thuki)"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    let builtin = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == crate::config::defaults::PROVIDER_ID_BUILTIN)
        .unwrap();
    assert_eq!(
        builtin.label,
        crate::config::defaults::DEFAULT_BUILTIN_LABEL
    );
}

#[test]
fn unknown_kind_provider_is_dropped_and_builtin_reseeded() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            [[inference.providers]]
            id = "weird"
            kind = "anthropic"
            label = "Cloud"
            base_url = "https://example.com"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert!(!c.inference.providers.iter().any(|p| p.id == "weird"));
    assert!(c
        .inference
        .providers
        .iter()
        .any(|p| p.kind == PROVIDER_KIND_BUILTIN));
}

#[test]
fn ollama_provider_with_empty_base_url_is_dropped_then_reseeded() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "   "
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.kind == PROVIDER_KIND_OLLAMA)
        .unwrap();
    assert_eq!(ollama.base_url, DEFAULT_OLLAMA_URL);
}

#[test]
fn ollama_provider_with_non_http_base_url_is_reset_to_default() {
    // Defense-in-depth: a non-http(s) scheme (or a scheme-less host) would be
    // POSTed verbatim by the backend, so the loader resets it to the default.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "file:///etc/passwd"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.kind == PROVIDER_KIND_OLLAMA)
        .unwrap();
    assert_eq!(ollama.base_url, DEFAULT_OLLAMA_URL);
}

#[test]
fn ollama_provider_with_https_base_url_is_preserved() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "https://ollama.example.com:11434"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.kind == PROVIDER_KIND_OLLAMA)
        .unwrap();
    assert_eq!(ollama.base_url, "https://ollama.example.com:11434");
}

#[test]
fn missing_builtin_provider_is_reseeded() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert!(c
        .inference
        .providers
        .iter()
        .any(|p| p.kind == PROVIDER_KIND_BUILTIN));
}

#[test]
fn new_shape_with_model_roundtrips_through_toml() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    let mut c = AppConfig::default();
    if let Some(p) = c
        .inference
        .providers
        .iter_mut()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
    {
        p.model = "llama3.1:8b".to_string();
    }
    atomic_write(&path, &c).unwrap();
    let reloaded = load_from_path(&path).unwrap();
    let ollama = reloaded
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
        .unwrap();
    assert_eq!(ollama.model, "llama3.1:8b");
    assert_eq!(
        reloaded.inference.active_provider,
        c.inference.active_provider
    );
}

// ── inference providers: pre-providers active pin ───────────────────────────

#[test]
fn pre_providers_file_with_url_pins_active_to_ollama() {
    // A pre-providers file carrying an ollama_url: the user runs Ollama, so
    // the active pointer must land on the Ollama provider regardless of the
    // compiled default, and the URL must survive the migration.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[inference]\nollama_url = \"http://10.0.0.5:11434\"\n",
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert_eq!(c.inference.active_provider, PROVIDER_ID_OLLAMA);
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
        .unwrap();
    assert_eq!(ollama.base_url, "http://10.0.0.5:11434");
}

#[test]
fn pre_providers_file_without_url_key_pins_active_to_ollama() {
    // A pre-providers file WITHOUT an ollama_url key (the user never changed
    // the URL) is still a pre-providers file: providers are reseeded and the
    // active pointer must land on the Ollama provider.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "[inference]\nnum_ctx = 4096\n").unwrap();
    let c = load_from_path(&path).unwrap();
    assert_eq!(c.inference.active_provider, PROVIDER_ID_OLLAMA);
    assert!(c
        .inference
        .providers
        .iter()
        .any(|p| p.kind == PROVIDER_KIND_BUILTIN));
    assert!(c
        .inference
        .providers
        .iter()
        .any(|p| p.kind == PROVIDER_KIND_OLLAMA));
    assert_eq!(c.inference.num_ctx, 4096);
}

#[test]
fn pre_providers_explicit_custom_active_keeps_it() {
    // An explicit active_provider naming the Ollama provider in a
    // pre-providers file survives resolution unchanged.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[inference]\nactive_provider = \"ollama\"\nollama_url = \"http://10.0.0.5:11434\"\n",
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert_eq!(c.inference.active_provider, PROVIDER_ID_OLLAMA);
}

#[test]
fn pre_providers_explicit_builtin_is_pinned_to_ollama() {
    // A pre-providers file cannot legitimately point at the built-in provider
    // (none existed when the file was written), so an explicit "builtin" is
    // overridden to the Ollama provider.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        "[inference]\nactive_provider = \"builtin\"\nnum_ctx = 4096\n",
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert_eq!(c.inference.active_provider, PROVIDER_ID_OLLAMA);
}

#[test]
fn new_shape_config_active_untouched() {
    // A new-shape file (explicit [[inference.providers]]) is never pinned:
    // an explicit "builtin" choice is respected.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "builtin"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert_eq!(c.inference.active_provider, PROVIDER_ID_BUILTIN);
}

#[test]
fn fresh_seed_uses_compiled_default() {
    // A fresh-seeded config (schema Default = default_providers()) is NOT a
    // pre-providers file: the compiled default pointer is left alone.
    let mut c = AppConfig::default();
    resolve(&mut c);
    assert_eq!(c.inference.active_provider, DEFAULT_ACTIVE_PROVIDER);
}

// ── inference providers: migrate helpers ─────────────────────────────────────

#[test]
fn attach_legacy_active_model_sets_model_on_active_provider() {
    // Legacy users (the only configs attach runs against) persisted
    // active = ollama; the fresh-install default is builtin now.
    let mut c = AppConfig::default();
    c.inference.active_provider = PROVIDER_ID_OLLAMA.to_string();
    assert!(attach_legacy_active_model(&mut c, Some("phi4:14b")));
    assert_eq!(c.inference.active_provider_model(), "phi4:14b");
    // idempotent: a second call with a different model does not overwrite
    assert!(!attach_legacy_active_model(&mut c, Some("other:1b")));
    assert_eq!(c.inference.active_provider_model(), "phi4:14b");
}

#[test]
fn attach_legacy_active_model_targets_the_active_provider_only() {
    // The legacy slug must land on the *active* provider, never on some other
    // provider that merely happens to have an empty model. Add a second
    // Ollama-kind provider, make it active (empty model), and give the default
    // Ollama entry a pre-existing model: attach writes the active one and
    // leaves the other untouched.
    let mut c = AppConfig::default();
    let mut remote = ollama_provider("http://10.0.0.9:11434");
    remote.id = "ollama-remote".to_string();
    c.inference.providers.push(remote);
    c.inference.active_provider = "ollama-remote".to_string();
    if let Some(ollama) = c
        .inference
        .providers
        .iter_mut()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
    {
        ollama.model = "existing:7b".to_string();
    }
    assert!(attach_legacy_active_model(&mut c, Some("legacy:1b")));
    let remote = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == "ollama-remote")
        .unwrap();
    assert_eq!(remote.model, "legacy:1b");
    let ollama = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_OLLAMA)
        .unwrap();
    assert_eq!(ollama.model, "existing:7b");
}

#[test]
fn legacy_model_attaches_only_to_ollama_kind_provider() {
    // The legacy SQLite slug is by definition an Ollama model name. When the
    // active provider is not Ollama-kind (post-flip: a fresh builtin default),
    // the slug must NOT attach: writing an Ollama slug onto the built-in
    // provider would make chat fail with ModelNotFound.
    let mut c = AppConfig::default();
    c.inference.active_provider = PROVIDER_ID_BUILTIN.to_string();
    assert!(!attach_legacy_active_model(&mut c, Some("phi4:14b")));
    let builtin = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == PROVIDER_ID_BUILTIN)
        .unwrap();
    assert!(builtin.model.is_empty());

    // Active = Ollama-kind with an empty model: attaches as before.
    let mut c = AppConfig::default();
    c.inference.active_provider = PROVIDER_ID_OLLAMA.to_string();
    assert!(attach_legacy_active_model(&mut c, Some("phi4:14b")));
    assert_eq!(c.inference.active_provider_model(), "phi4:14b");
}

#[test]
fn attach_legacy_active_model_ignores_empty_and_missing_provider() {
    let mut c = AppConfig::default();
    assert!(!attach_legacy_active_model(&mut c, None));
    assert!(!attach_legacy_active_model(&mut c, Some("   ")));
    assert_eq!(c.inference.active_provider_model(), "");

    // No matching active provider -> no-op (defensive).
    let mut orphan = AppConfig::default();
    orphan.inference.active_provider = "ghost".to_string();
    assert!(!attach_legacy_active_model(&mut orphan, Some("x")));
}

#[test]
fn toml_has_providers_detects_shape() {
    assert!(!toml_has_providers(
        "[inference]\nollama_url = \"http://x\"\n"
    ));
    assert!(toml_has_providers(
        "[inference]\nactive_provider=\"ollama\"\n[[inference.providers]]\nid=\"ollama\"\nkind=\"ollama\"\nbase_url=\"http://x\"\n"
    ));
    assert!(!toml_has_providers("not valid toml ["));
    assert!(!toml_has_providers("[inference]\n"));
}

#[test]
fn provider_struct_default_is_all_empty() {
    let p = Provider::default();
    assert!(p.id.is_empty());
    assert!(p.kind.is_empty());
    assert!(p.base_url.is_empty());
    assert!(p.model.is_empty());
    assert!(!p.vision);
}

// ── inference providers: openai kind ────────────────────────────────────────

#[test]
fn openai_provider_constructor_shape() {
    let p = openai_provider("lmstudio", "LM Studio", "http://localhost:1234");
    assert_eq!(p.id, "lmstudio");
    assert_eq!(p.kind, PROVIDER_KIND_OPENAI);
    assert_eq!(p.label, "LM Studio");
    assert_eq!(p.base_url, "http://localhost:1234");
    assert!(p.model.is_empty());
    assert!(!p.vision);
}

#[test]
fn openai_kind_with_url_is_kept() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "lmstudio"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
            [[inference.providers]]
            id = "lmstudio"
            kind = "openai"
            label = "LM Studio"
            base_url = "http://localhost:1234"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    let p = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == "lmstudio")
        .expect("openai provider should be retained");
    assert_eq!(p.kind, PROVIDER_KIND_OPENAI);
    assert_eq!(p.base_url, "http://localhost:1234");
}

#[test]
fn openai_kind_without_url_is_dropped() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
            [[inference.providers]]
            id = "lmstudio"
            kind = "openai"
            label = "LM Studio"
            base_url = ""
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert!(
        !c.inference.providers.iter().any(|p| p.id == "lmstudio"),
        "openai provider with empty base_url must be dropped"
    );
}

#[test]
fn openai_kind_bad_scheme_is_dropped() {
    // Both a non-http(s) scheme and a bare host without a scheme are rejected.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
            [[inference.providers]]
            id = "bad-scheme"
            kind = "openai"
            label = "Bad"
            base_url = "file:///x"
            [[inference.providers]]
            id = "no-scheme"
            kind = "openai"
            label = "No scheme"
            base_url = "localhost:1234"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert!(
        !c.inference.providers.iter().any(|p| p.id == "bad-scheme"),
        "openai provider with file:// scheme must be dropped"
    );
    assert!(
        !c.inference.providers.iter().any(|p| p.id == "no-scheme"),
        "openai provider with scheme-less host must be dropped"
    );
}

#[test]
fn provider_vision_flag_roundtrips() {
    // A TOML file with vision=true on an openai provider survives load unmodified.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "jan"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
            [[inference.providers]]
            id = "jan"
            kind = "openai"
            label = "Jan"
            base_url = "http://localhost:1337"
            vision = true
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    let jan = c
        .inference
        .providers
        .iter()
        .find(|p| p.id == "jan")
        .expect("jan provider must be retained");
    assert!(jan.vision, "vision=true must round-trip through TOML load");
}

#[test]
fn unknown_kind_still_dropped() {
    // Regression: adding openai must not affect the unknown-kind drop path.
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            [inference]
            active_provider = "ollama"
            [[inference.providers]]
            id = "builtin"
            kind = "builtin"
            label = "Built-in"
            [[inference.providers]]
            id = "ollama"
            kind = "ollama"
            label = "Ollama"
            base_url = "http://127.0.0.1:11434"
            [[inference.providers]]
            id = "cloud"
            kind = "anthropic"
            label = "Cloud"
            base_url = "https://api.anthropic.com"
        "#,
    )
    .unwrap();
    let c = load_from_path(&path).unwrap();
    assert!(
        !c.inference.providers.iter().any(|p| p.id == "cloud"),
        "provider with unknown kind must still be dropped"
    );
}
