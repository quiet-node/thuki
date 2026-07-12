/**
 * Ask-bar globe control for `behavior.auto_search` (design A2).
 *
 * Tinted when auto search is on; muted outline when on-demand. Writes the same
 * config field as Settings › Behavior via `set_config_field`, so the two
 * surfaces stay one bit. Optimistic local state rolls back if the write fails.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { useConfig } from '../contexts/ConfigContext';
import { Tooltip } from './Tooltip';

/** Hoisted globe SVG so the button never re-allocates icon JSX. */
const GLOBE_ICON = (
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <circle
      cx="12"
      cy="12"
      r="9"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
    />
    <path
      d="M3 12h18M12 3a14 14 0 010 18M12 3a14 14 0 000 18"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
    />
  </svg>
);

/**
 * Tooltip copy for the current mode. Teaches the off-path without a settings trip.
 *
 * @param on Whether auto search is currently enabled.
 */
function tooltipFor(on: boolean): string {
  return on
    ? 'Auto search · on · click for /search only'
    : 'On demand · /search only · click for Auto';
}

/**
 * Accessible name for the switch (screen readers).
 *
 * @param on Whether auto search is currently enabled.
 */
function ariaLabelFor(on: boolean): string {
  return on
    ? 'Auto search on. Click to require /search for the web.'
    : 'On demand search. Click to enable auto search.';
}

export interface AutoSearchGlobeButtonProps {
  /**
   * When true, the control is non-interactive (e.g. while a generation is
   * busy). Defaults to false.
   */
  disabled?: boolean;
}

/**
 * Globe icon toggle for auto vs on-demand web search.
 *
 * @param disabled When true, clicks are ignored and the button is dimmed.
 */
export function AutoSearchGlobeButton({
  disabled = false,
}: AutoSearchGlobeButtonProps) {
  const config = useConfig();
  const [autoSearch, setAutoSearch] = useState(config.behavior.autoSearch);
  /** Disables the button while a write is in flight (re-renders). */
  const [pending, setPending] = useState(false);

  // Stay aligned with Settings (or another window) via config-updated hydrate.
  /* eslint-disable @eslint-react/set-state-in-effect -- mirror external config writes (Settings / other webview) into local optimistic state */
  useEffect(() => {
    setAutoSearch(config.behavior.autoSearch);
  }, [config.behavior.autoSearch]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  /**
   * Flips `behavior.auto_search` on disk and in managed state.
   * Optimistic UI; restores the prior value if the invoke fails.
   * The button is `disabled` while busy or when the host passes `disabled`,
   * so this handler is only reached for an enabled click (no inner guard).
   */
  const handleToggle = useCallback(async () => {
    const next = !autoSearch;
    const previous = autoSearch;
    setAutoSearch(next);
    setPending(true);
    try {
      await invoke('set_config_field', {
        section: 'behavior',
        key: 'auto_search',
        value: next,
      });
    } catch {
      // Boundary: IPC / allowlist / disk failure. Revert optimistic flip.
      setAutoSearch(previous);
    } finally {
      setPending(false);
    }
  }, [autoSearch]);

  const on = autoSearch;
  const isDisabled = disabled || pending;
  return (
    <Tooltip label={tooltipFor(on)}>
      <button
        type="button"
        onClick={() => {
          void handleToggle();
        }}
        disabled={isDisabled}
        aria-label={ariaLabelFor(on)}
        aria-pressed={on}
        data-testid="auto-search-globe"
        className={`shrink-0 w-7 h-7 flex items-center justify-center rounded-lg transition-colors duration-150 outline-none cursor-pointer disabled:opacity-40 disabled:cursor-default ${
          on
            ? 'text-primary hover:bg-primary/10'
            : 'text-text-secondary/55 hover:text-text-secondary hover:bg-primary/8 ring-1 ring-inset ring-white/10'
        }`}
      >
        {GLOBE_ICON}
      </button>
    </Tooltip>
  );
}
