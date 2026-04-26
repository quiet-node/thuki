import { useEffect, useMemo, useRef, useState } from 'react';
import type { ModelCapabilitiesMap } from '../types/model';

const CHECK_ICON_PATH = (
  <path
    d="M3 8l3.5 3.5L13 5"
    stroke="currentColor"
    strokeWidth="2.2"
    strokeLinecap="round"
    strokeLinejoin="round"
  />
);

const LISTBOX_ID = 'thuki-model-picker-listbox';

/**
 * Builds the capability caption rendered beneath each picker row's model
 * name. Always leads with "text" (every chat-completion model handles
 * text), then appends "vision" and/or "thinking" when the model supports
 * them. Returns `null` only when capabilities for the model are unknown
 * (not yet loaded), which lets the caller suppress the caption line
 * entirely during cold start.
 *
 * Exported for direct unit testing.
 */
export function formatCapabilityLabel(
  capabilities: ModelCapabilitiesMap | undefined,
  model: string,
): string | null {
  const caps = capabilities?.[model];
  if (!caps) return null;
  const flags: string[] = ['text'];
  if (caps.vision) flags.push('vision');
  if (caps.thinking) flags.push('thinking');
  return flags.join(' · ');
}

/** Props for the {@link ModelPickerPanel} content panel. */
export interface ModelPickerPanelProps {
  /** Full list of available model slugs. */
  models: string[];
  /** Currently active model slug. */
  activeModel: string;
  /** Called with the chosen slug when the user clicks or keyboard-selects a row. */
  onSelect: (model: string) => void;
  /**
   * Called when the user presses Escape inside the panel. The host is
   * responsible for closing the drawer/dropdown in response.
   */
  onClose?: () => void;
  /**
   * Per-model capability map keyed by slug. When provided, each row
   * renders a small capability suffix ("· vision · thinking"). Omit or
   * pass an empty map to render plain rows (legacy / loading states).
   */
  capabilities?: ModelCapabilitiesMap;
}

/**
 * Inline model picker panel rendered as a drawer above the ask bar or as a
 * floating dropdown in chat mode.
 *
 * Combobox-style keyboard model: focus stays in the filter input, ArrowDown/
 * ArrowUp move the `aria-activedescendant` marker through the visible rows,
 * Enter commits the highlighted row, and Escape asks the host to close.
 */
export function ModelPickerPanel({
  models,
  activeModel,
  onSelect,
  onClose,
  capabilities,
}: ModelPickerPanelProps) {
  const [filter, setFilter] = useState('');
  const [highlightedIndex, setHighlightedIndex] = useState(0);
  const listboxRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(() => {
    const trimmed = filter.trim();
    if (trimmed === '') return models;
    const needle = trimmed.toLowerCase();
    return models.filter((m) => m.toLowerCase().includes(needle));
  }, [filter, models]);

  /* eslint-disable @eslint-react/set-state-in-effect -- canonical index-clamp
     when the filtered list shrinks; drives no secondary effects and React
     bails out of the rerender when the next state equals the previous. */
  useEffect(() => {
    if (filtered.length === 0) {
      setHighlightedIndex(0);
      return;
    }
    if (highlightedIndex >= filtered.length) {
      setHighlightedIndex(filtered.length - 1);
    }
  }, [filtered, highlightedIndex]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  const activeId =
    filtered.length > 0 && highlightedIndex < filtered.length
      ? `${LISTBOX_ID}-option-${highlightedIndex}`
      : undefined;

  // Keep the highlighted row visible when it scrolls off-view. scrollIntoView
  // is absent in happy-dom/jsdom; the optional call becomes a no-op there.
  useEffect(() => {
    if (!activeId) return;
    const el = listboxRef.current?.querySelector<HTMLElement>(`#${activeId}`);
    /* v8 ignore next -- scrollIntoView is a host API not available in jsdom */
    el?.scrollIntoView?.({ block: 'nearest' });
  }, [activeId]);

  const commit = (index: number) => {
    if (index < 0 || index >= filtered.length) return;
    onSelect(filtered[index]);
  };

  return (
    <div className="flex flex-col w-full">
      <div className="px-3 pt-3 pb-2 border-b border-surface-border">
        <input
          type="text"
          role="combobox"
          aria-controls={LISTBOX_ID}
          aria-expanded="true"
          aria-activedescendant={activeId}
          aria-autocomplete="list"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'ArrowDown') {
              e.preventDefault();
              if (filtered.length === 0) return;
              setHighlightedIndex((i) => (i + 1) % filtered.length);
              return;
            }
            if (e.key === 'ArrowUp') {
              e.preventDefault();
              if (filtered.length === 0) return;
              setHighlightedIndex(
                (i) => (i - 1 + filtered.length) % filtered.length,
              );
              return;
            }
            if (e.key === 'Home') {
              e.preventDefault();
              if (filtered.length > 0) setHighlightedIndex(0);
              return;
            }
            if (e.key === 'End') {
              e.preventDefault();
              if (filtered.length > 0) setHighlightedIndex(filtered.length - 1);
              return;
            }
            if (e.key === 'Enter') {
              e.preventDefault();
              commit(highlightedIndex);
              return;
            }
            if (e.key === 'Escape') {
              e.preventDefault();
              onClose?.();
              return;
            }
          }}
          placeholder="Filter models..."
          autoFocus
          className="w-full bg-transparent text-xs text-text-primary placeholder:text-text-secondary outline-none"
        />
      </div>

      <div
        ref={listboxRef}
        id={LISTBOX_ID}
        role="listbox"
        aria-label="Available models"
        className="overflow-y-auto py-1 max-h-[280px]"
      >
        {models.length === 0 ? (
          <p className="px-3 py-4 text-xs text-text-secondary text-center">
            No models available.
          </p>
        ) : filtered.length === 0 ? (
          <p className="px-3 py-4 text-xs text-text-secondary text-center">
            No models found.
          </p>
        ) : (
          filtered.map((model, index) => {
            const active = model === activeModel;
            const highlighted = index === highlightedIndex;
            const capLabel = formatCapabilityLabel(capabilities, model);
            return (
              <button
                key={model}
                id={`${LISTBOX_ID}-option-${index}`}
                type="button"
                role="option"
                aria-selected={active}
                aria-label={
                  capLabel
                    ? `${model}, ${capLabel.replace(/ · /g, ', ')}`
                    : model
                }
                tabIndex={-1}
                onMouseEnter={() => setHighlightedIndex(index)}
                onClick={() => commit(index)}
                className={`flex items-start justify-between gap-2.5 px-3 py-2 rounded-lg w-full text-left text-sm text-text-primary cursor-pointer transition-colors duration-120 ${
                  highlighted ? 'bg-white/5' : 'hover:bg-white/5'
                }`}
              >
                <span className="flex-1 min-w-0 flex flex-col gap-0.5">
                  <span className="overflow-hidden text-ellipsis whitespace-nowrap leading-tight">
                    {model}
                  </span>
                  {capLabel && (
                    <span
                      className="text-[10.5px] text-text-secondary leading-tight tracking-wide"
                      data-testid="model-capability-label"
                    >
                      {capLabel}
                    </span>
                  )}
                </span>
                <svg
                  className="w-3.5 h-3.5 shrink-0 mt-0.5 text-primary"
                  style={{ opacity: active ? 1 : 0 }}
                  viewBox="0 0 16 16"
                  fill="none"
                  xmlns="http://www.w3.org/2000/svg"
                  aria-hidden="true"
                >
                  {CHECK_ICON_PATH}
                </svg>
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}
