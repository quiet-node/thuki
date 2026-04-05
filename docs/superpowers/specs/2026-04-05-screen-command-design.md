# Design Spec: `/screen` Command and Slash Command System

**Date:** 2026-04-05
**Status:** Approved for implementation

---

## Overview

This spec covers three tightly related features:

1. A **slash command system** for the Thuki ask bar, including tab-completion UI
2. The **`/screen` command** — captures a screenshot at submit time and attaches it to the user's message as full-screen context for the AI
3. A **commands reference doc** (`docs/commands.md`) structured to grow as more commands are added

The core user experience: the user types `/screen explain this bug`, presses Enter, and Thuki silently captures the screen at that moment (excluding its own overlay via native macOS APIs), sends it to Ollama alongside the message, and the screenshot appears in the user's chat bubble just like a pasted image. No UI flicker, no window hide, no preview thumbnail before sending.

---

## Goals

- Full screen context awareness, triggered on demand, without sacrificing privacy
- Slash command UX that matches industry convention: commands are message-level directives typed at the start, executed on submit
- A command registry that scales to future commands with zero structural change
- Screenshot storage unified with existing image infrastructure — single folder, single lifecycle, same cleanup

---

## Non-Goals

- Automatic screenshot on every activation (privacy concern)
- Screenshot preview/thumbnail in the ask bar before sending
- More than one screenshot per message
- Any changes to `useOllama.ts`, `ask_ollama`, or the streaming pipeline
- Commands other than `/screen` in this iteration

---

## Architecture

Five components are added or modified:

```
src-tauri/src/screenshot.rs       NEW  — capture_screenshot Tauri command
src/config/commands.ts             NEW  — command registry (single source of truth)
src/components/CommandSuggestion.tsx  NEW  — tab-completion UI above ask bar
src/view/AskBarView.tsx           MOD  — wires up CommandSuggestion, enforces limits
src/App.tsx                       MOD  — submit-time /screen detection and capture
docs/commands.md                  NEW  — user-facing commands reference
```

Nothing else changes. `useOllama.ts`, `commands.rs`, `history.rs`, and `database.rs` are untouched. `images.rs` receives one constant update (`MAX_IMAGES_PER_MESSAGE`: 3 → 4) and no behavioral changes.

---

## Slash Command System

### Command Registry (`src/config/commands.ts`)

A single exported `COMMANDS` array is the source of truth for both the suggestion UI and the submit-time parser:

```ts
interface Command {
  trigger: string;       // e.g. "/screen"
  label: string;         // display name in suggestion row
  description: string;   // one-line description shown in suggestion row
  icon: string;          // icon identifier (maps to an SVG component)
}
```

Adding a future command means adding one entry here. Nothing else.

### Command Position

Commands are only valid at the **beginning of a message** (`query.trimStart().startsWith('/')`). Inline commands (`explain this /screen`) are not recognized and pass through as literal text. This matches the convention used by Cursor, GitHub Copilot Chat, and Claude Code.

### Submit-Time Parsing

`App.tsx` parses the query on submit:

1. Check if `query.trimStart()` starts with `/`
2. Extract the trigger token (first whitespace-delimited word)
3. Look it up in `COMMANDS`
4. If matched: strip the trigger from the display message, execute the command's side effect, then call `ask()`
5. If not matched: submit as-is (unknown `/foo` text passes through unchanged)

The parsed display message — with the trigger token stripped — is what appears in the chat bubble and is sent to Ollama.

---

## Tab-Completion UI (`CommandSuggestion.tsx`)

### Trigger Condition

The component renders when `query.trimStart()` starts with `/`. It dismisses when:
- The user presses Escape
- The user Backspaces past the `/` (query no longer starts with `/`)
- A command is selected (Tab or Enter on a highlighted row)
- The user clicks outside

### Visual Design

A frameless popover anchored to the **top edge of the ask bar, growing upward**. Not a separate modal. Shares the same NSPanel vibrancy background as the rest of the window.

Per row:
- Left: 28x28px rounded icon (outlined SVG, 14px, consistent stroke weight)
- Center-left: command trigger in regular weight (`/screen`)
- Center-right: muted description text in smaller size
- Right: `Tab` key badge on the currently highlighted row only

Header: `COMMANDS` in small all-caps muted label above the rows.

Maximum visible rows: 6 before scroll. At the current command count (1), the popover is minimal and unobtrusive.

### Keyboard Behavior

| Key | Action |
|-----|--------|
| Arrow Down / Up | Move highlight, wraps around |
| Tab | Complete highlighted command into input, dismiss popover |
| Enter (on highlighted row) | Same as Tab |
| Enter (no row highlighted, query starts with `/`) | Pass through to submit — unknown command |
| Escape | Dismiss popover, keep typed text |
| Backspace | Normal edit; dismisses popover if query no longer starts with `/` |

### Ghost Text

When one match remains, inline ghost text completes the trigger in the textarea: `/sc` renders as `/sc` (user-typed, orange) + `reen` (ghost, muted). Tab accepts it.

### No-Match State

A single muted "No commands found" row. Never an empty box.

---

## `/screen` Command

### Behavior

At submit time, when `/screen` is detected:

1. The trigger is stripped from the message: `/screen explain this` becomes `explain this`
2. `invoke('capture_screenshot')` is called — returns an absolute file path
3. The path is appended to `imagePaths` alongside any manually attached images
4. `ask(cleanMessage, quotedText, imagePaths)` is called as normal
5. The screenshot appears in the user's chat bubble as an image thumbnail, identical to pasted images

No preview before sending. No thumbnail in the ask bar. The ask bar is clean throughout.

### Screenshot Capture (`src-tauri/src/screenshot.rs`)

**macOS 14+ (primary path):** `SCScreenshotManager` with an `SCContentFilter` that excludes Thuki's own bundle ID. The filter is constructed before capture so Thuki's NSPanel is absent from the resulting image. No window hide, no flicker.

**macOS 12-13 (fallback):** `CGWindowListCreateImageFromArray`, passing all on-screen window IDs except Thuki's own `CGWindowID`. Also flicker-free.

The captured image is passed as raw bytes directly into `images::save_image(&base_dir, &raw_bytes)` — the same compression pipeline used for pasted images (JPEG, quality 85, max 1920px). The result is a UUID-named `.jpg` file in `<app_data_dir>/images/`.

**Required permission:** Screen Recording (`com.apple.security.screen-recording-description`). This is a new permission that Thuki does not currently require. The app must request it and handle the denied case gracefully.

**Error handling:** If the permission is denied or capture fails, `capture_screenshot` returns an `Err`. The frontend surfaces this as a standard error bubble in the conversation: `"Screen Recording permission is required to use /screen. Grant it in System Settings > Privacy & Security > Screen Recording."` The message is not submitted.

### Screenshot Storage and Lifecycle

Screenshots go into `<app_data_dir>/images/` — the same flat directory as pasted and dragged images. No separate folder, no separate cleanup.

- If the conversation is saved: the screenshot path is referenced in SQLite and retained by `cleanup_orphaned_images`
- If the conversation is not saved: `cleanup_orphaned_images` removes the file on next startup, exactly as it does for unsaved pasted images

No additional lifecycle code needed.

### Image Limits

| Slot | Limit | Enforcement |
|------|-------|-------------|
| Manual uploads (paste, drag, file picker) | 3 | `MAX_IMAGES = 3` constant in `AskBarView.tsx`; upload controls disabled at 3 |
| `/screen` capture | 1 | Only one `/screen` token is recognized per message; extras are ignored |
| Total per message | 4 | Combined at submit time in `App.tsx` |

The existing `MAX_IMAGES_PER_MESSAGE = 3` constant in `images.rs` is updated to 4 to reflect the new combined maximum. The frontend constant `MAX_IMAGES` stays at 3 and continues to gate manual uploads only.

---

## Permission Handling

Thuki currently requires Accessibility permission (for the CGEventTap hotkey listener). Screen Recording is a separate macOS permission category.

At first `/screen` use:
1. macOS automatically shows the system permission prompt if Screen Recording has not been granted
2. If the user grants it: capture proceeds normally on the next attempt (requires re-invocation; macOS does not retroactively grant mid-flight)
3. If the user denies it: `SCScreenshotManager` / `CGWindowListCreateImage` return an error; frontend shows the error bubble with instructions

No proactive permission pre-flight on app launch. The permission is requested lazily on first use.

---

## Documentation (`docs/commands.md`)

A new user-facing reference file. Structured with a consistent per-command section so future commands slot in:

```
# Commands

Commands are triggered by typing / at the start of a message...

## /screen
Description, behavior, notes, permission requirement.

(future commands follow the same section format)
```

This file replaces any inline documentation of the `/screen` feature scattered in other docs.

---

## Testing

### `screenshot.rs`
The `capture_screenshot` command is a thin OS-API wrapper and gets `#[cfg_attr(coverage_nightly, coverage(off))]` per the existing pattern. The path construction and temp naming logic are pure functions tested in isolation.

### `commands.ts`
- Registry is non-empty
- Each entry has all required fields
- No duplicate trigger strings
- All trigger strings start with `/`

### `CommandSuggestion.tsx`
- Renders when `query` starts with `/`
- Does not render when `query` does not start with `/`
- Filters correctly: `/sc` shows `/screen`, `/xyz` shows no-match state
- Tab on highlighted row updates `query` to the full trigger
- Escape dismisses without changing `query`
- Arrow keys move highlight; wraps at boundaries

### `AskBarView.tsx`
- Upload controls disable at 3 manual images
- Upload controls do not disable based on whether a `/screen` capture will be added
- `MAX_IMAGES` guard allows 3 manual images regardless of `/screen` state

### `App.tsx`
- `/screen` at start of message: `capture_screenshot` is invoked, returned path is appended to `imagePaths`, trigger token is stripped from display message
- `/screen` not at start: treated as literal text, `capture_screenshot` is not invoked
- `capture_screenshot` error: error bubble shown, `ask()` is not called
- Manual images + `/screen`: both path sets are merged correctly before calling `ask()`

---

## Open Questions (resolved)

| Question | Decision |
|----------|----------|
| Capture timing | At submit, not at command selection |
| Screenshot in ask bar thumbnail? | No — appears in chat bubble after send |
| Storage location | `<app_data_dir>/images/` — unified with pasted images |
| Naming in UI | "Commands" (header), "slash commands" (docs/onboarding) |
| Command position | Beginning of message only |
| Image limit | 3 manual + 1 screen = 4 total |
| Tab completion style | Suggestion chip above input, option B |
| macOS 15 compatibility | Not an issue — Thuki is taking the screenshot (filter-based exclusion), not hiding from others |
