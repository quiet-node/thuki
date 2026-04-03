/* v8 ignore file -- type-only declarations, no runtime code */

/**
 * TypeScript mirror of the Rust `ConversationSummary` struct in `database.rs`.
 * Used for rendering conversation list items in the history panel.
 */
export interface ConversationSummary {
  /** UUID primary key. */
  id: string;
  /** AI-generated or placeholder title. Null until a title is set. */
  title: string | null;
  /** Ollama model name used for this conversation. */
  model: string;
  /** Unix timestamp (seconds) of the last message. */
  updated_at: number;
  /** Total number of messages in this conversation. */
  message_count: number;
}

/**
 * TypeScript mirror of the Rust `PersistedMessage` struct in `database.rs`.
 * Returned by `load_conversation` when restoring a saved session.
 */
export interface PersistedMessage {
  /** UUID primary key. */
  id: string;
  /** `'user'` or `'assistant'`. */
  role: string;
  /** Full message content. */
  content: string;
  /** Quoted host-app text attached to this message, if any. */
  quoted_text: string | null;
  /** Unix timestamp (seconds) the message was created. */
  created_at: number;
}

/**
 * Response shape returned by the `save_conversation` Tauri command.
 */
export interface SaveConversationResponse {
  conversation_id: string;
}

/**
 * Message payload shape expected by the `save_conversation` and
 * `generate_title` Tauri commands.
 */
export interface SaveMessagePayload {
  role: string;
  content: string;
  quoted_text: string | null;
}
