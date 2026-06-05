import { Tooltip } from './Tooltip';

interface ReplaceButtonProps {
  /** Rewritten text to write back into the source app. */
  content: string;
  /**
   * Writes `content` into the source app, replacing the user's selection. The
   * paste lands in the source app while the overlay stays open, so the user
   * can replace repeatedly.
   */
  onReplace: (text: string) => void;
}

/**
 * Icon-only button rendered below a `/rewrite` or `/refine` result. Writes the
 * rewritten text back into the source app, replacing the user's selection. A
 * hover tooltip (the same `Tooltip` the chat header icons use) names the
 * action, since the button carries only an icon.
 */
export function ReplaceButton({ content, onReplace }: ReplaceButtonProps) {
  return (
    <Tooltip label="Replace selection">
      <button
        onClick={() => onReplace(content)}
        className="transition-opacity duration-150 text-white/40 hover:text-white/70 p-0.5 rounded cursor-pointer shrink-0 flex"
        aria-label="Replace selection in source app"
      >
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden="true"
        >
          <rect x="3" y="4" width="12" height="16" rx="2" />
          <line x1="21" y1="12" x2="9" y2="12" />
          <polyline points="13 8 9 12 13 16" />
        </svg>
      </button>
    </Tooltip>
  );
}
