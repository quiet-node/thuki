/**
 * CommandSuggestion — slash command autocomplete popover.
 *
 * Renders above the ask bar when the user types a "/" prefix.
 * The parent (AskBarView) is responsible for computing `filteredCommands`
 * and managing `highlightedIndex`. This component is purely presentational.
 */

import type React from 'react';
import type { Command } from '../config/commands';

/** Hoisted static screen-capture SVG icon. */
const SCREEN_ICON = (
  <svg
    width="14"
    height="14"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect
      x="1"
      y="2"
      width="14"
      height="10"
      rx="1.5"
      stroke="currentColor"
      strokeWidth="1.5"
    />
    <path
      d="M5 14h6"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
    <path
      d="M8 12v2"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
  </svg>
);

/** Returns the icon for a given command trigger. Currently all commands use SCREEN_ICON. */
function iconForTrigger(trigger: string): React.ReactNode {
  switch (trigger) {
    case '/screen':
    default:
      return SCREEN_ICON;
  }
}

interface CommandSuggestionProps {
  /** Filtered list of matching commands to display (computed by parent). */
  commands: readonly Command[];
  /** Index of the currently highlighted row (-1 means nothing highlighted). */
  highlightedIndex: number;
  /** Called with the trigger string when a row is clicked. */
  onSelect: (trigger: string) => void;
}

/**
 * Renders the slash command suggestion popover.
 *
 * When `commands` is empty, shows a "No commands found" placeholder row.
 * Otherwise renders one row per command with an icon, label, description,
 * and a Tab badge on the highlighted row.
 */
export function CommandSuggestion({
  commands,
  highlightedIndex,
  onSelect,
}: CommandSuggestionProps) {
  return (
    <div
      className="mb-1 rounded-xl border border-surface-border bg-surface-base backdrop-blur-2xl shadow-bar overflow-hidden"
      role="listbox"
      aria-label="Command suggestions"
    >
      {/* Header */}
      <div className="px-3 pt-2 pb-1">
        <span className="text-[10px] font-semibold tracking-widest text-text-secondary uppercase">
          Commands
        </span>
      </div>

      {commands.length === 0 ? (
        <div className="px-3 pb-2 text-sm text-text-secondary italic">
          No commands found
        </div>
      ) : (
        <ul className="pb-1" role="presentation">
          {commands.map((cmd, index) => {
            const isHighlighted = index === highlightedIndex;
            return (
              <li
                key={cmd.trigger}
                role="option"
                aria-selected={isHighlighted}
                className={`flex items-center gap-2.5 px-3 py-1.5 cursor-pointer select-none transition-colors duration-100 ${
                  isHighlighted
                    ? 'bg-white/8 text-text-primary'
                    : 'text-text-secondary hover:bg-white/5 hover:text-text-primary'
                }`}
                onMouseDown={(e) => {
                  // Use mousedown + preventDefault so the textarea doesn't lose
                  // focus before the click is registered.
                  e.preventDefault();
                  onSelect(cmd.trigger);
                }}
              >
                {/* Icon */}
                <span
                  className={`shrink-0 ${isHighlighted ? 'text-primary' : ''}`}
                >
                  {iconForTrigger(cmd.trigger)}
                </span>

                {/* Trigger label */}
                <span className="text-sm font-medium text-text-primary shrink-0">
                  {cmd.label}
                </span>

                {/* Description */}
                <span className="text-xs text-text-secondary min-w-0 truncate flex-1">
                  {cmd.description}
                </span>

                {/* Tab badge on highlighted row only */}
                {isHighlighted && (
                  <span className="shrink-0 text-[10px] font-medium text-text-secondary border border-surface-border rounded px-1 py-0.5 leading-none">
                    Tab
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
