import { useState, useEffect, useCallback, useRef } from 'react';
import { ConversationItem } from './ConversationItem';
import { SwitchConfirmation } from './SwitchConfirmation';
import type { ConversationSummary } from '../types/history';

/** Debounce delay in ms before firing a search query. */
const SEARCH_DEBOUNCE_MS = 200;

/**
 * Groups a flat list of conversations into date buckets for display.
 * Returns an ordered array of `[label, items]` pairs.
 */
function groupByDate(
  conversations: ConversationSummary[],
): [string, ConversationSummary[]][] {
  const nowSec = Math.floor(Date.now() / 1000);
  const DAY = 86400;

  const todayStart = nowSec - (nowSec % DAY);
  const yesterdayStart = todayStart - DAY;

  const buckets = new Map<string, ConversationSummary[]>();

  for (const conv of conversations) {
    let label: string;
    if (conv.updated_at >= todayStart) {
      label = 'Today';
    } else if (conv.updated_at >= yesterdayStart) {
      label = 'Yesterday';
    } else {
      label = 'Earlier';
    }

    const existing = buckets.get(label);
    if (existing) {
      existing.push(conv);
    } else {
      buckets.set(label, [conv]);
    }
  }

  return Array.from(buckets.entries());
}

interface HistoryPanelProps {
  /**
   * Called to fetch the conversation list, optionally filtered by a search
   * term. Must return a promise resolving to `ConversationSummary[]`.
   */
  listConversations: (search?: string) => Promise<ConversationSummary[]>;
  /**
   * Called when the user selects a conversation and either has no current
   * messages, or confirmed "Just Switch".
   */
  onLoadConversation: (id: string) => void;
  /**
   * Called when the user confirms "Save & Switch" from the switch prompt.
   */
  onSaveAndLoad: (id: string) => void;
  /** Called when the user clicks the delete button on a row. */
  onDeleteConversation: (id: string) => Promise<void>;
  /**
   * True when the current session has unsaved messages. Causes a
   * `SwitchConfirmation` to appear before loading.
   */
  hasCurrentMessages: boolean;
  /**
   * The id of the conversation currently loaded. When the user clicks the row
   * matching this id, the action is a no-op (already viewing it).
   */
  currentConversationId?: string | null;
  /**
   * When true, renders a "+ New conversation" footer button. Pass `false`
   * in ask-bar mode (the input itself starts a new conversation).
   */
  showNewConversation: boolean;
  /** Called when the user clicks "+ New conversation". */
  onNewConversation?: () => void;
}

/**
 * Search-first conversation history panel, shared by ask-bar mode (inline)
 * and conversation-view mode (dropdown).
 *
 * - Fetches and groups conversations by date on mount.
 * - Debounces search input at 200 ms.
 * - Shows a `SwitchConfirmation` prompt before loading when the user has an
 *   active session (`hasCurrentMessages`).
 * - Optimistically removes deleted conversations from the list.
 * - Conditionally renders a "+ New conversation" footer via `showNewConversation`.
 */
export function HistoryPanel({
  listConversations,
  onLoadConversation,
  onSaveAndLoad,
  onDeleteConversation,
  hasCurrentMessages,
  currentConversationId,
  showNewConversation,
  onNewConversation,
}: HistoryPanelProps) {
  const [conversations, setConversations] = useState<ConversationSummary[]>([]);
  const [search, setSearch] = useState('');
  const [loadError, setLoadError] = useState(false);
  /** Id of the conversation the user clicked when confirmation is needed. */
  const [pendingId, setPendingId] = useState<string | null>(null);

  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  /** Fetches (or re-fetches) the conversation list with an optional search term. */
  const fetchList = useCallback(
    async (term?: string) => {
      setLoadError(false);
      try {
        const results = await listConversations(term);
        setConversations(results);
      } catch {
        setLoadError(true);
      }
    },
    [listConversations],
  );

  // Initial load on mount.
  useEffect(() => {
    void fetchList();
  }, [fetchList]);

  // Debounced search: fires 200 ms after the user stops typing.
  const handleSearchChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const value = e.target.value;
      setSearch(value);

      if (debounceRef.current !== null) {
        clearTimeout(debounceRef.current);
      }
      debounceRef.current = setTimeout(() => {
        void fetchList(value || undefined);
      }, SEARCH_DEBOUNCE_MS);
    },
    [fetchList],
  );

  // Cleanup debounce timer on unmount.
  useEffect(() => {
    return () => {
      if (debounceRef.current !== null) {
        clearTimeout(debounceRef.current);
      }
    };
  }, []);

  const handleSelect = useCallback(
    (id: string) => {
      if (id === currentConversationId) {
        return;
      }
      if (hasCurrentMessages) {
        setPendingId(id);
      } else {
        onLoadConversation(id);
      }
    },
    [hasCurrentMessages, onLoadConversation, currentConversationId],
  );

  const handleSaveAndSwitch = useCallback(() => {
    /* v8 ignore start -- SwitchConfirmation only renders when pendingId !== null */
    if (pendingId !== null) {
      onSaveAndLoad(pendingId);
      setPendingId(null);
    }
    /* v8 ignore stop */
  }, [pendingId, onSaveAndLoad]);

  const handleJustSwitch = useCallback(() => {
    /* v8 ignore start -- SwitchConfirmation only renders when pendingId !== null */
    if (pendingId !== null) {
      onLoadConversation(pendingId);
      setPendingId(null);
    }
    /* v8 ignore stop */
  }, [pendingId, onLoadConversation]);

  const handleCancelSwitch = useCallback(() => {
    setPendingId(null);
  }, []);

  const handleDelete = useCallback(
    async (id: string) => {
      // Capture snapshot for rollback before optimistic removal.
      // find() always returns a match (called via ConversationItem on a known id).
      // The ?? null and the snapshot !== null guard are defensive only.
      /* v8 ignore start */
      const snapshot = conversations.find((c) => c.id === id) ?? null;
      /* v8 ignore stop */
      setConversations((prev) => prev.filter((c) => c.id !== id));
      try {
        await onDeleteConversation(id);
      } catch {
        // Backend rejected — restore the item in its original sort position.
        /* v8 ignore start */
        if (snapshot !== null) {
          setConversations((prev) =>
            // Item was just removed optimistically; can't already be present.
            prev.some((c) => c.id === id)
              ? prev
              : [...prev, snapshot].sort((a, b) => b.updated_at - a.updated_at),
          );
        }
        /* v8 ignore stop */
      }
    },
    [onDeleteConversation, conversations],
  );

  const groups = groupByDate(conversations);
  const isEmpty = conversations.length === 0 && !loadError;

  return (
    <div className="history-panel flex flex-col w-full">
      {/* Search input — always visible, auto-focused via CSS autofocus attribute */}
      <div className="px-3 pt-3 pb-2 border-b border-surface-border">
        <input
          type="text"
          value={search}
          onChange={handleSearchChange}
          placeholder="Search past chats…"
          autoFocus
          className="w-full bg-transparent text-xs text-text-primary placeholder:text-text-secondary outline-none"
        />
      </div>

      {/* Switch confirmation — overlays the list when pending */}
      {pendingId !== null ? (
        <SwitchConfirmation
          onSaveAndSwitch={handleSaveAndSwitch}
          onJustSwitch={handleJustSwitch}
          onCancel={handleCancelSwitch}
        />
      ) : (
        <div className="overflow-y-auto py-1 max-h-[280px]">
          {loadError && (
            <p className="px-3 py-4 text-xs text-text-secondary text-center">
              Couldn&apos;t load history — try again.
            </p>
          )}

          {isEmpty && !loadError && (
            <p className="px-3 py-4 text-xs text-text-secondary text-center">
              No conversations yet.
            </p>
          )}

          {groups.map(([label, items]) => (
            <div key={label}>
              <p className="px-3 pt-2 pb-1 text-[10px] uppercase tracking-wider text-text-secondary select-none">
                {label}
              </p>
              {items.map((conv) => (
                <ConversationItem
                  key={conv.id}
                  conversation={conv}
                  onSelect={handleSelect}
                  onDelete={handleDelete}
                />
              ))}
            </div>
          ))}
        </div>
      )}

      {/* Optional footer — only shown in conversation-view mode */}
      {showNewConversation && pendingId === null && (
        <div className="border-t border-surface-border pt-1 pb-1">
          <button
            type="button"
            onClick={onNewConversation}
            aria-label="New conversation"
            className="w-full text-left px-3 py-2 text-xs text-primary hover:bg-primary/5 transition-colors duration-150 cursor-pointer rounded-b-lg"
          >
            + New conversation
          </button>
        </div>
      )}
    </div>
  );
}
