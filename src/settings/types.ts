/**
 * Settings panel type definitions.
 *
 * These mirror the Rust `AppConfig` schema (snake_case) byte-for-byte so the
 * frontend can pass values straight through `set_config_field` without an
 * intermediate camelCase translation. Keeping the snake_case shape in the
 * Settings UI is intentional: the Settings GUI is a thin layer over the TOML
 * file (P4), and TOML keys are the user's mental model.
 *
 * Shared with the rest of the React tree's camelCase `AppConfig` (in
 * `contexts/ConfigContext`) only by value at the IPC boundary; the two
 * shapes are not interchangeable.
 */

/** One entry in the `[[inference.providers]]` array (snake_case, from TOML). */
export interface RawProvider {
  id: string;
  kind: string;
  label: string;
  base_url: string;
  model: string;
  /** Manual vision flag for `openai`-kind providers. Always `false` for `builtin` and `ollama`. */
  vision: boolean;
}

export interface RawAppConfig {
  inference: {
    active_provider: string;
    keep_warm_inactivity_minutes: number;
    num_ctx: number;
    providers: RawProvider[];
  };
  prompt: {
    system: string;
  };
  window: {
    overlay_width: number;
    max_chat_height: number;
    max_images: number;
    text_base_px: number;
    text_line_height: number;
    text_letter_spacing_px: number;
    text_font_weight: number;
  };
  quote: {
    max_display_lines: number;
    max_display_chars: number;
    max_context_length: number;
  };
  behavior: {
    auto_replace: boolean;
    auto_close: boolean;
    /** When true, built-in auto-search may open the web on plain turns. */
    auto_search: boolean;
    /** When true, first-use web-search notice has been dismissed forever. */
    search_notice_acknowledged: boolean;
    /** When true, completed turns auto-persist to SQLite history. */
    auto_save_conversations: boolean;
    /**
     * Days to keep saved conversations by last activity; `-1` forever.
     * Finite values prune older rows after confirm / at startup.
     */
    history_retention_days: number;
    /** When true, one-shot auto-save chat notice has been dismissed forever. */
    auto_save_notice_acknowledged: boolean;
  };
  debug: {
    trace_enabled: boolean;
    trace_retention_days: number;
  };
}

/** Tagged union returned by the Rust `set_config_field` command on failure. */
export type ConfigError =
  | { kind: 'seed_failed'; path: string; source: string }
  | { kind: 'io_error'; path: string; source: string }
  | { kind: 'unknown_section'; section: string }
  | { kind: 'unknown_field'; section: string; key: string }
  | { kind: 'type_mismatch'; section: string; key: string; message: string }
  | { kind: 'parse'; path: string; message: string };

/** Recovery marker payload returned by `get_corrupt_marker`. */
export interface CorruptMarker {
  path: string;
  ts: number;
}

/** Identifier for the active Settings tab. */
export type SettingsTabId =
  | 'general'
  | 'behavior'
  | 'display'
  | 'changelog'
  | 'about';

/**
 * Returns a human-friendly description of a Tauri-side `ConfigError`. Used
 * as the label inside inline `rowError` pills and the corrupt-recovery
 * banner. Centralized so the wording is consistent across every form row.
 */
export function describeConfigError(err: unknown): string {
  if (typeof err !== 'object' || err === null) {
    return 'Couldn’t save. Please try again.';
  }
  const e = err as Partial<ConfigError> & { kind?: string; message?: string };
  switch (e.kind) {
    case 'io_error':
      return `Couldn’t save: ${e.source ?? 'I/O error'}.`;
    case 'unknown_section':
      return `Unknown section: ${e.section}.`;
    case 'unknown_field':
      return `Unknown field: ${e.section}.${e.key}.`;
    case 'type_mismatch':
      return e.message ?? 'Wrong type for this field.';
    case 'parse':
      return 'config.toml has a syntax error. Restart Thuki to recover.';
    case 'seed_failed':
      return `Couldn’t write defaults: ${e.source ?? ''}.`;
    default:
      return typeof e.message === 'string' ? e.message : 'Couldn’t save.';
  }
}
