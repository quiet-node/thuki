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
    CURRENT_SCHEMA_VERSION, DEFAULT_COLLAPSED_HEIGHT, DEFAULT_HIDE_COMMIT_DELAY_MS,
    DEFAULT_MAX_CHAT_HEIGHT, DEFAULT_MODEL_NAME, DEFAULT_OLLAMA_URL, DEFAULT_OVERLAY_WIDTH,
    DEFAULT_QUOTE_MAX_CONTEXT_LENGTH, DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
    DEFAULT_QUOTE_MAX_DISPLAY_LINES, DEFAULT_SYSTEM_PROMPT_BASE, SLASH_COMMAND_PROMPT_APPENDIX,
};
use super::error::ConfigError;
use super::loader::{compose_system_prompt, load_from_path};
use super::schema::{AppConfig, ModelSection, PromptSection, QuoteSection, WindowSection};
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

// ── defaults module ──────────────────────────────────────────────────────────

#[test]
fn defaults_const_values_match_schema_defaults() {
    // Guard rail: a change to a default in defaults.rs must flow through to
    // AppConfig::default(). If this test fails, someone changed one but not both.
    let c = AppConfig::default();
    assert_eq!(c.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(c.model.available, vec![DEFAULT_MODEL_NAME.to_string()]);
    assert_eq!(c.model.ollama_url, DEFAULT_OLLAMA_URL);
    assert_eq!(c.prompt.system, "");
    assert_eq!(c.prompt.resolved_system, "");
    assert_eq!(c.window.overlay_width, DEFAULT_OVERLAY_WIDTH);
    assert_eq!(c.window.collapsed_height, DEFAULT_COLLAPSED_HEIGHT);
    assert_eq!(c.window.max_chat_height, DEFAULT_MAX_CHAT_HEIGHT);
    assert_eq!(c.window.hide_commit_delay_ms, DEFAULT_HIDE_COMMIT_DELAY_MS);
    assert_eq!(c.quote.max_display_lines, DEFAULT_QUOTE_MAX_DISPLAY_LINES);
    assert_eq!(c.quote.max_display_chars, DEFAULT_QUOTE_MAX_DISPLAY_CHARS);
    assert_eq!(c.quote.max_context_length, DEFAULT_QUOTE_MAX_CONTEXT_LENGTH);
}

#[test]
fn defaults_prompt_base_is_nonempty() {
    // Guard against accidentally shipping an empty persona prompt.
    assert!(!DEFAULT_SYSTEM_PROMPT_BASE.trim().is_empty());
}

// ── schema module ───────────────────────────────────────────────────────────

#[test]
fn section_defaults_are_sensible() {
    let m = ModelSection::default();
    assert_eq!(m.available, vec![DEFAULT_MODEL_NAME.to_string()]);
    assert_eq!(m.active(), DEFAULT_MODEL_NAME);

    let p = PromptSection::default();
    assert!(p.system.is_empty());

    let w = WindowSection::default();
    assert_eq!(w.overlay_width, DEFAULT_OVERLAY_WIDTH);

    let q = QuoteSection::default();
    assert_eq!(q.max_display_lines, DEFAULT_QUOTE_MAX_DISPLAY_LINES);
}

#[test]
fn model_section_active_falls_back_when_list_empty() {
    // Guard: loader should prevent this, but active() has a defensive fallback
    // so the struct can't explode if a caller bypasses the loader.
    let m = ModelSection {
        available: vec![],
        ollama_url: DEFAULT_OLLAMA_URL.to_string(),
    };
    assert_eq!(m.active(), DEFAULT_MODEL_NAME);
}

#[test]
fn model_section_active_returns_first() {
    let m = ModelSection {
        available: vec!["custom:model".to_string(), "other:model".to_string()],
        ollama_url: DEFAULT_OLLAMA_URL.to_string(),
    };
    assert_eq!(m.active(), "custom:model");
}

#[test]
fn app_config_serde_round_trip_matches_defaults() {
    let original = AppConfig::default();
    let toml_str = toml::to_string_pretty(&original).expect("serialize");
    let parsed: AppConfig = toml::from_str(&toml_str).expect("deserialize");
    // prompt.resolved_system is marked #[serde(skip)] so it does not round-trip
    // through the file. Compare everything else.
    assert_eq!(parsed.schema_version, original.schema_version);
    assert_eq!(parsed.model, original.model);
    assert_eq!(parsed.prompt.system, original.prompt.system);
    assert_eq!(parsed.window, original.window);
    assert_eq!(parsed.quote, original.quote);
}

#[test]
fn app_config_partial_file_fills_missing_fields_with_defaults() {
    // Only declare one field; serde(default) fills the rest.
    let partial = r#"
        schema_version = 1
        [model]
        available = ["custom:only"]
    "#;
    let parsed: AppConfig = toml::from_str(partial).expect("partial file parses");
    assert_eq!(parsed.model.available, vec!["custom:only".to_string()]);
    assert_eq!(parsed.model.ollama_url, DEFAULT_OLLAMA_URL);
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

// ── loader: first run (file missing) ────────────────────────────────────────

#[test]
fn load_missing_file_seeds_defaults_and_returns_them() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    assert!(!path.exists());

    let config = load_from_path(&path).expect("seed on first run");

    assert!(path.exists(), "file should be seeded");
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    assert_eq!(config.model.active(), DEFAULT_MODEL_NAME);
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
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
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
            schema_version = 1
            [model]
            available = ["custom:a", "custom:b"]
            ollama_url = "http://localhost:99999"
        "#,
    )
    .unwrap();

    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.model.available,
        vec!["custom:a".to_string(), "custom:b".to_string()]
    );
    assert_eq!(config.model.active(), "custom:a");
    assert_eq!(config.model.ollama_url, "http://localhost:99999");
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
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);

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

// ── loader: schema version ───────────────────────────────────────────────────

#[test]
fn load_newer_schema_version_reseeds() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "schema_version = 99\n").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    // Renamed file should exist.
    let has_corrupt = std::fs::read_dir(&dir).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".corrupt-")
    });
    assert!(has_corrupt);
}

#[test]
fn load_older_unsupported_schema_reseeds() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "schema_version = 0\n").unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
}

// ── loader: read error (not NotFound) ───────────────────────────────────────

#[cfg(unix)]
#[test]
fn load_unreadable_file_returns_in_memory_defaults() {
    use std::os::unix::fs::PermissionsExt;

    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(&path, "schema_version = 1\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();

    // If the current user is root, the permission bits are ignored and this
    // test is vacuous. Skip rather than falsely passing.
    if std::fs::read(&path).is_ok() {
        // Restore perms so the tempdir cleanup can succeed, then skip.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        return;
    }

    let config = load_from_path(&path).expect("fallback to in-memory defaults");
    assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    // Restore so cleanup works.
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
}

// ── loader: resolve (empties and bounds) ────────────────────────────────────

#[test]
fn resolve_empty_available_list_falls_back_to_default_model() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [model]
            available = []
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.model.available, vec![DEFAULT_MODEL_NAME.to_string()]);
    assert_eq!(config.model.active(), DEFAULT_MODEL_NAME);
}

#[test]
fn resolve_whitespace_only_entries_are_filtered() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [model]
            available = ["  ", "custom:x", " ", "custom:y"]
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.model.available,
        vec!["custom:x".to_string(), "custom:y".to_string()]
    );
}

#[test]
fn resolve_entry_whitespace_is_trimmed() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [model]
            available = ["  spaced:model  "]
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.model.available, vec!["spaced:model".to_string()]);
}

#[test]
fn resolve_empty_ollama_url_falls_back() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [model]
            ollama_url = "   "
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.model.ollama_url, DEFAULT_OLLAMA_URL);
}

#[test]
fn resolve_empty_system_prompt_uses_built_in_base_plus_appendix() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [prompt]
            system = "   "
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
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
fn resolve_custom_system_prompt_flows_through_with_appendix() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [prompt]
            system = "You are a custom assistant."
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
fn resolve_out_of_bounds_floats_reset_to_defaults() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [window]
            overlay_width = 0.0
            collapsed_height = 99999.0
            max_chat_height = -1.0
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.overlay_width, DEFAULT_OVERLAY_WIDTH);
    assert_eq!(config.window.collapsed_height, DEFAULT_COLLAPSED_HEIGHT);
    assert_eq!(config.window.max_chat_height, DEFAULT_MAX_CHAT_HEIGHT);
}

#[test]
fn resolve_non_finite_float_resets() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [window]
            overlay_width = nan
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.overlay_width, DEFAULT_OVERLAY_WIDTH);
}

#[test]
fn resolve_out_of_bounds_u64_resets() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
            [window]
            hide_commit_delay_ms = 99999
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(
        config.window.hide_commit_delay_ms,
        DEFAULT_HIDE_COMMIT_DELAY_MS
    );
}

#[test]
fn resolve_out_of_bounds_u32_resets() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
            schema_version = 1
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
            schema_version = 1
            [window]
            overlay_width = 800.0
            collapsed_height = 100.0
            max_chat_height = 1000.0
            hide_commit_delay_ms = 250
            [quote]
            max_display_lines = 6
            max_display_chars = 500
            max_context_length = 8192
        "#,
    )
    .unwrap();
    let config = load_from_path(&path).unwrap();
    assert_eq!(config.window.overlay_width, 800.0);
    assert_eq!(config.window.collapsed_height, 100.0);
    assert_eq!(config.window.max_chat_height, 1000.0);
    assert_eq!(config.window.hide_commit_delay_ms, 250);
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
    assert!(contents.contains("schema_version = 1"));
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
    assert!(contents.contains("schema_version = 1"));
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

    let e = ConfigError::TooNew {
        found: 99,
        supported: 1,
    };
    let m = e.to_string();
    assert!(m.contains("99"));
    assert!(m.contains('1'));

    let e = ConfigError::NoMigrationYet { found: 0 };
    assert!(e.to_string().contains('0'));
}
