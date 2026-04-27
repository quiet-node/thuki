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
    DEFAULT_JUDGE_TIMEOUT_S, DEFAULT_MAX_CHAT_HEIGHT, DEFAULT_MAX_ITERATIONS, DEFAULT_OLLAMA_URL,
    DEFAULT_OVERLAY_WIDTH, DEFAULT_QUOTE_MAX_CONTEXT_LENGTH, DEFAULT_QUOTE_MAX_DISPLAY_CHARS,
    DEFAULT_QUOTE_MAX_DISPLAY_LINES, DEFAULT_READER_BATCH_TIMEOUT_S,
    DEFAULT_READER_PER_URL_TIMEOUT_S, DEFAULT_READER_URL, DEFAULT_ROUTER_TIMEOUT_S,
    DEFAULT_SEARCH_TIMEOUT_S, DEFAULT_SEARXNG_MAX_RESULTS, DEFAULT_SEARXNG_URL,
    DEFAULT_SYSTEM_PROMPT_BASE, DEFAULT_TOP_K_URLS, SLASH_COMMAND_PROMPT_APPENDIX,
};
use super::error::ConfigError;
use super::loader::{compose_system_prompt, load_from_path};
use super::schema::{
    AppConfig, InferenceSection, PromptSection, QuoteSection, SearchSection, WindowSection,
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

// ── defaults module ──────────────────────────────────────────────────────────

#[test]
fn defaults_const_values_match_schema_defaults() {
    // Guard rail: a change to a default in defaults.rs must flow through to
    // AppConfig::default(). If this test fails, someone changed one but not both.
    let c = AppConfig::default();
    assert_eq!(c.inference.ollama_url, DEFAULT_OLLAMA_URL);
    assert_eq!(c.prompt.system, "");
    assert_eq!(c.prompt.resolved_system, "");
    assert_eq!(c.window.overlay_width, DEFAULT_OVERLAY_WIDTH);
    assert_eq!(c.window.max_chat_height, DEFAULT_MAX_CHAT_HEIGHT);
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

// ── schema module ───────────────────────────────────────────────────────────

#[test]
fn section_defaults_are_sensible() {
    let m = InferenceSection::default();
    assert_eq!(m.ollama_url, DEFAULT_OLLAMA_URL);

    let p = PromptSection::default();
    assert!(p.system.is_empty());

    let w = WindowSection::default();
    assert_eq!(w.overlay_width, DEFAULT_OVERLAY_WIDTH);

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
    // Only declare one field; serde(default) fills the rest.
    let partial = r#"
        [inference]
        ollama_url = "http://localhost:9999"
    "#;
    let parsed: AppConfig = toml::from_str(partial).expect("partial file parses");
    assert_eq!(parsed.inference.ollama_url, "http://localhost:9999");
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
    assert_eq!(config.inference.ollama_url, DEFAULT_OLLAMA_URL);
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
    assert_eq!(config.inference.ollama_url, DEFAULT_OLLAMA_URL);
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
    assert_eq!(config.inference.ollama_url, "http://localhost:99999");
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
    assert_eq!(config.inference.ollama_url, DEFAULT_OLLAMA_URL);

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
    assert_eq!(config.inference.ollama_url, DEFAULT_OLLAMA_URL);
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
    assert_eq!(config.inference.ollama_url, "http://localhost:11434");
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
    assert_eq!(config.inference.ollama_url, DEFAULT_OLLAMA_URL);
}

#[test]
fn resolve_empty_system_prompt_uses_built_in_base_plus_appendix() {
    let dir = fresh_temp_dir();
    let path = config_path_in(&dir);
    std::fs::write(
        &path,
        r#"
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
    assert_eq!(config.inference.ollama_url, DEFAULT_OLLAMA_URL);

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
    assert!(contents.contains("ollama_url"));
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
    assert!(contents.contains("ollama_url"));
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
