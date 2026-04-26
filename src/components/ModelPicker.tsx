const CHIP_ICON = (
  <svg
    width="14"
    height="14"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect
      x="3"
      y="3"
      width="10"
      height="10"
      rx="1.5"
      stroke="currentColor"
      strokeWidth="1.5"
    />
    <path
      d="M5 1V3M8 1V3M11 1V3M5 13V15M8 13V15M11 13V15M1 5H3M1 8H3M1 11H3M13 5H15M13 8H15M13 11H15"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
  </svg>
);

/** Props for the {@link ModelPicker} trigger button. */
export interface ModelPickerProps {
  /** Called when the user clicks the trigger to toggle the picker panel. */
  onClick: () => void;
  /** When true, the button is inert (e.g. during generation). */
  disabled: boolean;
  /** Reflects whether the picker panel is currently open (drives aria-expanded). */
  isOpen: boolean;
}

/**
 * Chip-style trigger button that opens/closes the model picker panel.
 *
 * The panel itself is rendered by App.tsx as an inline drawer (same
 * grow/shrink animation as the history panel) so the ResizeObserver drives
 * natural window growth without any portal or frame-manipulation logic.
 */
export function ModelPicker({ onClick, disabled, isOpen }: ModelPickerProps) {
  return (
    <button
      type="button"
      aria-label="Choose model"
      aria-expanded={isOpen}
      aria-haspopup="listbox"
      data-model-picker-toggle
      disabled={disabled}
      onClick={onClick}
      className="shrink-0 w-7 h-7 flex items-center justify-center rounded-lg text-text-secondary hover:text-primary hover:bg-primary/10 transition-colors duration-150 disabled:opacity-40 disabled:cursor-default cursor-pointer outline-none"
    >
      {CHIP_ICON}
    </button>
  );
}
