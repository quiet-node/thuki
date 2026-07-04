import { memo } from 'react';
import type { ConversationSummary } from '../types/history';
import { formatRelativeTime } from '../utils/formatRelativeTime';

/** Hoisted static delete icon - avoids re-allocation on every render. */
const DELETE_ICON = (
  <svg
    width="10"
    height="10"
    viewBox="0 0 10 10"
    fill="none"
    aria-hidden="true"
  >
    <path
      d="M1.5 1.5L8.5 8.5M8.5 1.5L1.5 8.5"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
  </svg>
);

interface ConversationItemProps {
  /** The conversation summary to render. */
  conversation: ConversationSummary;
  /** Called with the conversation id when the row is clicked. */
  onSelect: (id: string) => void;
  /** Called with the conversation id when the delete button is clicked. */
  onDelete: (id: string) => void;
  /** When true, renders the row with an active/highlighted style. */
  isActive?: boolean;
}

/**
 * Renders a single conversation row in the history panel.
 *
 * Displays the conversation title (falling back to "Untitled"), a relative
 * timestamp, and a delete button revealed on hover. The entire row is a
 * button for keyboard accessibility.
 */
export const ConversationItem = memo(function ConversationItem({
  conversation,
  onSelect,
  onDelete,
  isActive = false,
}: ConversationItemProps) {
  const title = conversation.title ?? 'Untitled';

  return (
    <div className="history-item group relative w-full">
      <button
        type="button"
        onClick={() => onSelect(conversation.id)}
        aria-label={title}
        aria-current={isActive ? 'true' : undefined}
        className={`relative w-full flex flex-col gap-0.5 text-left pl-3 pr-9 py-2 rounded-lg transition-colors duration-150 cursor-pointer hover:bg-white/5 ${
          isActive
            ? "before:absolute before:content-[''] before:left-0 before:top-1.5 before:bottom-1.5 before:w-[3px] before:rounded-r before:bg-primary before:shadow-[0_0_10px_rgba(255,141,92,0.5)]"
            : ''
        }`}
      >
        <span
          className={`text-xs truncate leading-snug ${isActive ? 'text-primary font-medium' : 'text-text-primary'}`}
        >
          {title}
        </span>
        <span className="text-[10px] text-text-secondary leading-none">
          {formatRelativeTime(conversation.updated_at)}
        </span>
      </button>

      {/* Delete: absolute overlay revealed on row hover, so it never reserves a
          layout column that would permanently narrow every title. */}
      <button
        type="button"
        onClick={() => onDelete(conversation.id)}
        aria-label="Delete conversation"
        className="absolute right-1.5 top-1/2 -translate-y-1/2 p-1 rounded text-text-secondary opacity-0 group-hover:opacity-100 hover:text-red-400 hover:bg-red-500/10 transition-opacity duration-150 cursor-pointer"
      >
        {DELETE_ICON}
      </button>
    </div>
  );
});
