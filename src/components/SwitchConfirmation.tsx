import { memo } from 'react';

interface SwitchConfirmationProps {
  /** Called when the user wants to save the current session then load the new one. */
  onSaveAndSwitch: () => void;
  /** Called when the user wants to discard the current session and load the new one. */
  onJustSwitch: () => void;
  /** Called when the user wants to go back without switching. */
  onCancel: () => void;
}

/**
 * Inline confirmation prompt displayed inside the history panel when the user
 * selects a conversation while an unsaved (or saved) session is active.
 *
 * Presents two primary actions:
 * - **Save & Switch** — persists the current conversation before loading.
 * - **Just Switch** — discards the current conversation and loads immediately.
 *
 * A **Cancel** action returns the user to the history list.
 */
export const SwitchConfirmation = memo(function SwitchConfirmation({
  onSaveAndSwitch,
  onJustSwitch,
  onCancel,
}: SwitchConfirmationProps) {
  return (
    <div className="px-3 py-3 flex flex-col gap-2.5">
      <p className="text-xs text-text-secondary leading-snug">
        Switch conversations?
      </p>

      <div className="flex flex-col gap-1.5">
        <button
          type="button"
          onClick={onSaveAndSwitch}
          className="w-full text-left px-3 py-2 rounded-lg text-xs font-medium bg-primary/10 text-primary hover:bg-primary/20 transition-colors duration-150 cursor-pointer"
        >
          Save &amp; Switch
        </button>

        <button
          type="button"
          onClick={onJustSwitch}
          className="w-full text-left px-3 py-2 rounded-lg text-xs text-text-primary hover:bg-white/5 transition-colors duration-150 cursor-pointer"
        >
          Just Switch
        </button>

        <button
          type="button"
          onClick={onCancel}
          aria-label="Cancel"
          className="w-full text-left px-3 py-2 rounded-lg text-xs text-text-secondary hover:bg-white/5 transition-colors duration-150 cursor-pointer"
        >
          Cancel
        </button>
      </div>
    </div>
  );
});
