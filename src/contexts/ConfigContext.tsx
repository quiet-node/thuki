/**
 * Application configuration context.
 *
 * Hydrates once from the Rust-side `get_config` Tauri command on mount, then
 * provides a synchronous `useConfig` hook to every descendant. Render is
 * gated until the first fetch resolves so components never see a null
 * config: this eliminates the per-call-site fallback literals that the
 * backend migration is specifically trying to kill.
 *
 * The Rust `AppConfig` serializes with snake_case field names (matching the
 * on-disk TOML schema). We translate to camelCase here so React components
 * keep their idiomatic JS names.
 */

import { createContext, use, useEffect, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Shape returned by the Rust `get_config` command (snake_case). */
interface RawAppConfig {
  schema_version: number;
  model: {
    available: string[];
    ollama_url: string;
  };
  prompt: {
    system: string;
  };
  window: {
    overlay_width: number;
    collapsed_height: number;
    max_chat_height: number;
    hide_commit_delay_ms: number;
  };
  quote: {
    max_display_lines: number;
    max_display_chars: number;
    max_context_length: number;
  };
}

/** Camel-cased, frontend-friendly view of the configuration. */
export interface AppConfig {
  schemaVersion: number;
  model: {
    /** First entry of `available` (the list-order invariant). */
    active: string;
    /** Full list, in order. */
    available: string[];
    ollamaUrl: string;
  };
  prompt: {
    /** Raw user-editable persona prompt (may be empty). */
    system: string;
  };
  window: {
    overlayWidth: number;
    collapsedHeight: number;
    maxChatHeight: number;
    hideCommitDelayMs: number;
  };
  quote: {
    maxDisplayLines: number;
    maxDisplayChars: number;
    maxContextLength: number;
  };
}

function transform(raw: RawAppConfig): AppConfig {
  return {
    schemaVersion: raw.schema_version,
    model: {
      active: raw.model.available[0] ?? '',
      available: raw.model.available,
      ollamaUrl: raw.model.ollama_url,
    },
    prompt: {
      system: raw.prompt.system,
    },
    window: {
      overlayWidth: raw.window.overlay_width,
      collapsedHeight: raw.window.collapsed_height,
      maxChatHeight: raw.window.max_chat_height,
      hideCommitDelayMs: raw.window.hide_commit_delay_ms,
    },
    quote: {
      maxDisplayLines: raw.quote.max_display_lines,
      maxDisplayChars: raw.quote.max_display_chars,
      maxContextLength: raw.quote.max_context_length,
    },
  };
}

const ConfigContext = createContext<AppConfig | null>(null);

/**
 * Renders children only once `get_config` resolves. Blocks with `null`
 * (no visible splash) for the tiny IPC round-trip; Tauri local IPC is
 * sub-10ms in practice.
 */
export function ConfigProvider({ children }: { children: ReactNode }) {
  const [config, setConfig] = useState<AppConfig | null>(null);

  useEffect(() => {
    void invoke<RawAppConfig>('get_config').then((raw) => {
      // In production Rust always returns a fully-populated RawAppConfig.
      // A nullish response here only happens in tests where `invoke` is mocked
      // without a handler for `get_config`; we fall back to DEFAULT_CONFIG so
      // the tree still mounts instead of spinning on a null state.
      if (raw == null) {
        setConfig(DEFAULT_CONFIG);
        return;
      }
      setConfig(transform(raw));
    });
  }, []);

  if (!config) return null;

  return <ConfigContext value={config}>{children}</ConfigContext>;
}

/**
 * Returns the current resolved `AppConfig`.
 *
 * When no `ConfigProvider` wraps the calling component, falls back to
 * `DEFAULT_CONFIG`. In production `main.tsx` always wraps `<App />`, so this
 * path only fires from component tests that render a leaf without setting up
 * a provider. Keeps test infrastructure minimal without compromising the
 * production single-source-of-truth guarantee.
 *
 * If test-side defaults ever drift from the Rust-side `AppConfig::default()`,
 * the fix is to update `DEFAULT_CONFIG` below. The two shapes are kept in
 * sync by hand because cross-language codegen is not worth the dependency
 * in a macOS-only desktop app.
 */
export function useConfig(): AppConfig {
  const value = use(ConfigContext);
  return value ?? DEFAULT_CONFIG;
}

/**
 * Test helper: wraps children with a synchronous (no `invoke`) ConfigContext
 * populated from `value`. Useful when a test needs to assert behavior against
 * a non-default config.
 */
export function ConfigProviderForTest({
  value,
  children,
}: {
  value: AppConfig;
  children: ReactNode;
}) {
  return <ConfigContext value={value}>{children}</ConfigContext>;
}

/**
 * Default AppConfig used when no `ConfigProvider` wraps the caller. Values
 * mirror the Rust-side `AppConfig::default()` (see
 * `src-tauri/src/config/defaults.rs`).
 */
export const DEFAULT_CONFIG: AppConfig = {
  schemaVersion: 1,
  model: {
    active: 'gemma4:e2b',
    available: ['gemma4:e2b'],
    ollamaUrl: 'http://127.0.0.1:11434',
  },
  prompt: { system: '' },
  window: {
    overlayWidth: 600,
    collapsedHeight: 80,
    maxChatHeight: 648,
    hideCommitDelayMs: 350,
  },
  quote: {
    maxDisplayLines: 4,
    maxDisplayChars: 300,
    maxContextLength: 4096,
  },
};
