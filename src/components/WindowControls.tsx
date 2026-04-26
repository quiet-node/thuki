/**
 * macOS-style traffic light window controls.
 *
 * Renders a thin header bar with close/minimize/zoom dots on the left.
 * On hover, the close button reveals an × icon; minimize and zoom dots
 * remain grayed as decorative elements (panel windows do not support
 * minimize or fullscreen).
 *
 * Window dragging is handled by the application root container via event
 * bubbling - mousedown events from the bar surface propagate up naturally.
 * A subtle divider at the bottom visually separates the controls from
 * the chat messages area below.
 */

import { memo } from 'react';
import { Tooltip } from './Tooltip';

/** Hoisted bookmark icon - save/saved state toggled via fill class. */
const BOOKMARK_ICON_EMPTY = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
  </svg>
);

const BOOKMARK_ICON_FILLED = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="currentColor"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
  </svg>
);

/** Hoisted new-conversation (plus) icon. */
const NEW_CONVERSATION_ICON = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <line x1="12" y1="5" x2="12" y2="19" />
    <line x1="5" y1="12" x2="19" y2="12" />
  </svg>
);

/** Hoisted chip icon for the active-model pill trigger. */
const CHIP_ICON = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 16 16"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.5"
    strokeLinecap="round"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect x="3" y="3" width="10" height="10" rx="1.5" />
    <path d="M5 1V3M8 1V3M11 1V3M5 13V15M8 13V15M11 13V15M1 5H3M1 8H3M1 11H3M13 5H15M13 8H15M13 11H15" />
  </svg>
);

/** Hoisted history (clock) icon. */
const HISTORY_ICON = (
  <svg
    width="13"
    height="13"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <circle cx="12" cy="12" r="10" />
    <polyline points="12 6 12 12 16 14" />
  </svg>
);

interface WindowControlsProps {
  /** Triggers the overlay hide animation sequence. */
  onClose: () => void;
  /**
   * Called when the user clicks the bookmark (save) icon.
   * Omit to hide the save button entirely.
   */
  onSave?: () => void;
  /**
   * True when the conversation has been saved to SQLite.
   * Renders the bookmark in its filled/confirmed state and disables the button.
   */
  isSaved?: boolean;
  /**
   * True when there is at least one completed AI response to save.
   * When false, the save button is disabled.
   */
  canSave?: boolean;
  /**
   * Called when the user clicks the "History ▾" button.
   * Omit to hide the history button entirely.
   */
  onHistoryOpen?: () => void;
  /**
   * Called when the user clicks the new-conversation (+) button.
   * Omit to hide the button entirely.
   */
  onNewConversation?: () => void;
  /**
   * Currently active model slug displayed in the pill trigger. When `null`
   * or `undefined` the pill renders a "Pick a model" placeholder so the
   * affordance stays visible: the picker is the recovery path when no
   * model is selected, so it must be reachable even with a null active.
   */
  activeModel?: string | null;
  /**
   * Called when the user clicks the active-model pill to open/close the picker.
   * Omit to hide the pill entirely. When provided the pill always renders,
   * regardless of `activeModel`, so users can recover from a no-model state.
   */
  onModelPickerToggle?: () => void;
  /** Drives `aria-expanded` on the pill button. */
  isModelPickerOpen?: boolean;
}

/** Decorative dot color for inactive buttons. */
const INACTIVE_DOT = 'rgba(255, 255, 255, 0.12)';

export const WindowControls = memo(function WindowControls({
  onClose,
  onSave,
  isSaved = false,
  canSave = false,
  onHistoryOpen,
  onNewConversation,
  activeModel,
  onModelPickerToggle,
  isModelPickerOpen = false,
}: WindowControlsProps) {
  // Disabled only when there is nothing to save yet and the conversation hasn't
  // been saved. Once saved the button stays active so the user can unsave.
  const saveDisabled = !isSaved && !canSave;

  return (
    <div className="shrink-0">
      <div className="group flex items-center px-4 py-2.5">
        {/* Close button - reveals × icon on group hover.
            Padding enlarges the hit area to ~24×24px without changing the
            12×12px visual dot; negative margin preserves flex spacing. */}
        <button
          type="button"
          onClick={onClose}
          className="group/close-btn p-1.5 -m-1.5 flex items-center justify-center rounded-full cursor-pointer"
          aria-label="Close window"
        >
          <div className="w-3 h-3 rounded-full bg-[#FF5F57] flex items-center justify-center transition-transform duration-150 group-hover/close-btn:scale-125 group-active/close-btn:scale-90">
            <svg
              width="6"
              height="6"
              viewBox="0 0 6 6"
              className="opacity-0 group-hover:opacity-100 transition-opacity duration-150"
              aria-hidden="true"
            >
              <path
                d="M0.5 0.5L5.5 5.5M5.5 0.5L0.5 5.5"
                stroke="rgba(0,0,0,0.6)"
                strokeWidth="1.2"
                strokeLinecap="round"
              />
            </svg>
          </div>
        </button>

        {/* Minimize - decorative only */}
        <div
          className="w-3 h-3 rounded-full ml-2"
          style={{ backgroundColor: INACTIVE_DOT }}
          aria-hidden="true"
        />

        {/* Zoom - decorative only */}
        <div
          className="w-3 h-3 rounded-full ml-2"
          style={{ backgroundColor: INACTIVE_DOT }}
          aria-hidden="true"
        />

        {/* Right-side header controls */}
        <div className="ml-auto flex items-center gap-1">
          {/* Active model pill trigger: leftmost, before save. The pill
              renders whenever the picker callback is wired up, regardless of
              whether a model is currently selected. When no model is active
              the chip surfaces the "Pick a model" affordance so the user
              has a one-click recovery path out of the no-model state. */}
          {onModelPickerToggle !== undefined && (
            <Tooltip label="Choose model">
              <button
                type="button"
                aria-label="Choose model"
                aria-expanded={isModelPickerOpen}
                aria-haspopup="listbox"
                data-model-picker-toggle
                onClick={onModelPickerToggle}
                className={`group/pill flex items-center gap-1.5 px-2 h-7 rounded-lg text-xs transition-colors duration-150 cursor-pointer ${
                  isModelPickerOpen ? 'bg-primary/10' : 'hover:bg-primary/8'
                }`}
              >
                <span
                  className={`shrink-0 transition-colors duration-150 ${
                    isModelPickerOpen
                      ? 'text-primary'
                      : 'text-text-secondary group-hover/pill:text-primary'
                  }`}
                >
                  {CHIP_ICON}
                </span>
                <span
                  className={`max-w-[120px] truncate transition-colors duration-150 ${
                    isModelPickerOpen
                      ? 'text-text-primary'
                      : 'text-text-secondary group-hover/pill:text-text-primary'
                  }`}
                >
                  {activeModel != null && activeModel.length > 0
                    ? activeModel
                    : 'Pick a model'}
                </span>
              </button>
            </Tooltip>
          )}

          {onSave !== undefined && (
            <Tooltip
              label={isSaved ? 'Remove from history' : 'Save conversation'}
            >
              <button
                type="button"
                onClick={onSave}
                disabled={saveDisabled}
                aria-label={
                  isSaved ? 'Remove from history' : 'Save conversation'
                }
                className={`w-7 h-7 flex items-center justify-center rounded-lg transition-colors duration-150 cursor-pointer disabled:cursor-default ${
                  isSaved
                    ? 'text-primary hover:text-text-secondary hover:bg-white/5'
                    : canSave
                      ? 'text-text-secondary hover:text-primary hover:bg-primary/8'
                      : 'text-text-secondary opacity-30'
                }`}
              >
                {isSaved ? BOOKMARK_ICON_FILLED : BOOKMARK_ICON_EMPTY}
              </button>
            </Tooltip>
          )}

          {onNewConversation !== undefined && (
            <Tooltip label="New conversation">
              <button
                type="button"
                onClick={onNewConversation}
                aria-label="New conversation"
                data-history-toggle
                className="w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-primary hover:bg-primary/8 transition-colors duration-150 cursor-pointer"
              >
                {NEW_CONVERSATION_ICON}
              </button>
            </Tooltip>
          )}

          {onHistoryOpen !== undefined && (
            <Tooltip label="Conversation history">
              <button
                type="button"
                onClick={onHistoryOpen}
                aria-label="Open history"
                data-history-toggle
                className="w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-primary hover:bg-primary/8 transition-colors duration-150 cursor-pointer"
              >
                {HISTORY_ICON}
              </button>
            </Tooltip>
          )}
        </div>
      </div>

      {/* Divider between controls and chat area */}
      <div className="h-px bg-surface-border" />
    </div>
  );
});
