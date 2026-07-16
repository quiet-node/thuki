import type { Message } from '../hooks/useModel';
import type { SaveOptions } from '../hooks/useConversationHistory';

/**
 * Drop streaming placeholder assistants from a create-on-submit payload so
 * SQLite does not store empty assistant shells mid-stream.
 *
 * Keeps every user row plus any assistant that already has visible body,
 * thinking, or an error stamp (prior completed turns still on screen).
 *
 * @param msgs Live transcript snapshot.
 * @returns Messages safe to bulk-insert on first create.
 */
export function messagesForCreateSave(msgs: Message[]): Message[] {
  return msgs.filter(
    (m) =>
      m.role === 'user' ||
      m.content.length > 0 ||
      Boolean(m.thinkingContent) ||
      Boolean(m.errorKind),
  );
}

/**
 * Dependencies for {@link createConversationOnSubmit}. Injected so the write
 * body is unit-testable without mounting App.
 */
export type CreateOnSubmitDeps = {
  /** Sync conversation UUID; null when still unsaved. */
  getConversationId: () => string | null;
  /** Live auto-save toggle. */
  isAutoSaveOn: () => boolean;
  /** Active model slug; null blocks create. */
  getModel: () => string | null;
  /** Transcript to persist (already filtered via {@link messagesForCreateSave}). */
  messages: Message[];
  /**
   * First-save bulk insert. Create always passes `{ generateTitle: false }`
   * so the title LLM waits for the first Done.
   */
  save: (
    messages: Message[],
    model: string,
    options?: SaveOptions,
  ) => Promise<void>;
  /** Record user ids already written so Done can skip re-insert. */
  onUserPersisted: (userMessageId: string) => void;
  /** Show the one-shot auto-save notice when still unacked. */
  onShowNotice: () => void;
  /** Whether the notice was already acknowledged. */
  isNoticeAcked: () => boolean;
};

/**
 * Create-on-submit write body: bulk-insert the user half (and any prior
 * completed turns) when auto-save is on and no conversation id exists yet.
 *
 * Safe to schedule on the history write chain; re-checks identity and toggle
 * so a concurrent winner or a mid-flight settings flip becomes a no-op.
 *
 * Does not fire title generation (stream may still be live). Marks users
 * persisted only when `save` actually stamped an identity.
 *
 * @param deps Injected getters and side effects.
 */
export async function createConversationOnSubmit(
  deps: CreateOnSubmitDeps,
): Promise<void> {
  if (deps.getConversationId() != null) return;
  if (!deps.isAutoSaveOn()) return;
  const model = deps.getModel();
  if (model == null) return;
  if (!deps.messages.some((m) => m.role === 'user')) return;

  await deps.save(deps.messages, model, { generateTitle: false });

  // Identity must be set after save; otherwise users are not in SQLite and
  // Done must not skip re-inserting them via the autoSaved set.
  if (deps.getConversationId() == null) return;

  for (const m of deps.messages) {
    if (m.role === 'user') {
      deps.onUserPersisted(m.id);
    }
  }

  if (!deps.isNoticeAcked()) {
    deps.onShowNotice();
  }
}
