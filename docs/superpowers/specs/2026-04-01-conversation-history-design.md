# Conversation History — Design Spec

**Date:** 2026-04-01
**Status:** Draft
**Depends on:** PR #16 (multi-turn conversation via /api/chat) — merged

## Problem

Thuki is a floating macOS secretary activated by double-tapping Command. Conversations are currently ephemeral — stored only in memory (Rust `Mutex<Vec<ChatMessage>>` + React `useState<Message[]>`). Everything is wiped on overlay close or app restart. Users have no way to revisit past conversations.

## Solution

Add opt-in conversation persistence with a dropdown history UI. Conversations are ephemeral by default — users explicitly save conversations worth keeping via a save button. This prevents quick one-shot Q&As from cluttering the history.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Storage engine | SQLite via `tauri-plugin-sql` | Industry standard for desktop apps. Cursor, Open WebUI use it. Handles millions of rows. |
| Storage location | `~/.thuki/thuki.db` | Consistent with CLI tool conventions (~/.claude, ~/.gemini). Easy to find, backup, reason about. |
| History UI | Dropdown/popover from header icon | Preserves Thuki's compact spotlight feel. History accessible but hidden by default. |
| Save behavior | Opt-in (explicit save button) | Thuki is for quick answers. Most interactions don't need persistence. Keeps history intentional. |
| Title generation | AI-generated via Ollama on save | Background request after save: "Summarize in 5 words or fewer." First-message preview as placeholder. |
| Activation behavior | Always start fresh | Double-tap Command opens empty input bar. Users pick past conversations from dropdown. |
| Conversation cap | None — unlimited | SQLite handles 100K+ conversations trivially. Add cleanup options later if needed. |
| Conversation deletion | Delete only (no archive) | Hover-reveal trash icon per item in dropdown. |
| Search | Basic title filter in v1 | Search field at top of dropdown filters by title substring. No FTS5. |

## Schema

```sql
PRAGMA journal_mode = WAL;

CREATE TABLE conversations (
    id          TEXT PRIMARY KEY,   -- UUID
    title       TEXT,               -- AI-generated or placeholder
    model       TEXT NOT NULL,      -- e.g. "llama3.2:3b"
    created_at  INTEGER NOT NULL,   -- unix timestamp ms
    updated_at  INTEGER NOT NULL,   -- unix timestamp ms
    meta        TEXT                -- JSON blob for future extensibility
);

CREATE TABLE messages (
    id              TEXT PRIMARY KEY,   -- UUID
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    role            TEXT NOT NULL,      -- 'user' | 'assistant'
    content         TEXT NOT NULL,
    quoted_text     TEXT,               -- optional context from host app
    created_at      INTEGER NOT NULL    -- unix timestamp ms
);

CREATE INDEX idx_messages_conversation ON messages(conversation_id, created_at);
CREATE INDEX idx_conversations_updated ON conversations(updated_at DESC);
```

## Data Flow

### Normal Usage (ephemeral — no save)

1. User double-taps Command, types a question
2. Frontend calls `ask_ollama`, streaming works as today
3. Messages live in React state + Rust `ConversationHistory`
4. User activates Thuki again or closes overlay — conversation is gone

### Save Flow

1. User has a conversation worth keeping
2. Taps the save icon (bookmark) in the chat header
3. Frontend calls a new Tauri command `save_conversation`
4. Backend creates a `conversations` row + writes all current messages to `messages` table
5. Conversation ID is stored in frontend state — conversation is now "saved"
6. Background: fires Ollama request to generate a title, updates `conversations.title` on completion
7. Subsequent messages in this saved conversation auto-persist on each completed exchange

### Loading a Past Conversation

1. User clicks history icon in header
2. Dropdown opens, reads conversation list from SQLite (sorted by `updated_at DESC`)
3. Search field filters by title substring
4. User clicks a conversation
5. Frontend reads messages from SQLite via `load_conversation` command
6. Messages populate React state
7. Backend `ConversationHistory` is synced with loaded messages
8. User can continue the conversation — new messages auto-persist

### Deleting a Conversation

1. User hovers over a conversation in the dropdown
2. Trash icon appears
3. Click triggers `delete_conversation` command
4. Backend deletes conversation + cascading messages from SQLite
5. Dropdown refreshes

## UI Spec

### Save Button

- Appears in the chat header area once `messages.length >= 2` (at least one exchange)
- Icon: bookmark outline (unsaved) / filled bookmark (saved)
- Position: right side of the chat header, near the existing controls
- Clicking toggles the conversation to "saved" state
- Visual feedback: icon fills in, brief animation

### History Dropdown

- **Trigger**: clock or hamburger icon next to the Thuki logo in the input bar header
- **Position**: drops down from the icon, left-aligned
- **Width**: ~260px
- **Max height**: ~360px with scroll
- **Contents** (top to bottom):
  1. Search input field with placeholder "Search conversations..."
  2. "+ New conversation" button (green accent)
  3. Scrollable list of saved conversations
- **Each conversation item**:
  - Title (truncated with ellipsis)
  - Relative timestamp ("2m", "1h", "Yesterday")
  - Hover: reveals trash icon on the right
- **Empty state**: "No saved conversations yet"

### Interaction States

- **Fresh activation**: empty input bar, no conversation loaded, save button hidden
- **In conversation (unsaved)**: messages visible, save button shows (outline)
- **In conversation (saved)**: messages visible, save button filled, new messages auto-persist
- **Viewing history**: dropdown open over the chat, clicking outside closes it
- **Loading past conversation**: messages populate, save button shows filled, can continue chatting

## New Tauri Commands

| Command | Params | Returns | Description |
|---------|--------|---------|-------------|
| `save_conversation` | `messages: Vec<Message>`, `model: String` | `conversation_id: String` | Creates conversation + writes all messages |
| `persist_message` | `conversation_id: String`, `message: Message` | `()` | Appends a single message to a saved conversation |
| `list_conversations` | `search: Option<String>` | `Vec<ConversationSummary>` | Lists conversations, optional title filter |
| `load_conversation` | `conversation_id: String` | `Vec<Message>` | Reads all messages for a conversation |
| `delete_conversation` | `conversation_id: String` | `()` | Deletes conversation + cascading messages |
| `generate_title` | `conversation_id: String`, `messages: Vec<Message>` | `()` | Background: asks Ollama for title, updates DB |

### ConversationSummary

```rust
struct ConversationSummary {
    id: String,
    title: Option<String>,
    model: String,
    updated_at: i64,
    message_count: i64,
}
```

## Frontend Changes

### New State in `useOllama` (or new hook)

- `conversationId: string | null` — null when unsaved, set after save
- `isSaved: boolean` — drives save button appearance

### New Hook: `useConversationHistory`

- `conversations: ConversationSummary[]` — list for dropdown
- `searchQuery: string` — filter input
- `loadConversation(id: string)` — loads messages, syncs backend
- `deleteConversation(id: string)` — removes from DB + list
- `saveConversation()` — persists current messages
- `refreshConversations()` — re-reads from DB

### New Components

- `HistoryDropdown` — the popover with search + conversation list
- `SaveButton` — bookmark icon in chat header
- `ConversationItem` — single row in the dropdown list

### Modified Components

- `App.tsx` — integrates history dropdown trigger, save button, conversation loading
- `useOllama.ts` — adds `conversationId` tracking, auto-persist logic for saved conversations

## Directory Structure (new files)

```
~/.thuki/
  thuki.db

src/
  components/
    HistoryDropdown.tsx
    SaveButton.tsx
    ConversationItem.tsx
  hooks/
    useConversationHistory.ts

src-tauri/src/
    database.rs          -- SQLite setup, migrations, queries
    commands.rs          -- new commands added here (or split to history_commands.rs)
```

## Out of Scope

- Archive functionality
- Folders, tags, or pinning
- FTS5 full-text search (title substring filter only)
- Conversation branching or forking
- Export/import
- Auto-save logic or smart thresholds
- Conversation cap or auto-cleanup
