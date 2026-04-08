# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

All commands use **Bun** as the package manager.

```bash
bun install              # Install dependencies
bun run dev              # Start Tauri dev server with HMR frontend
bun run frontend:dev     # Vite-only dev server (port 1420)

bun run build:frontend   # Typecheck + Vite build → dist/
bun run build:backend    # Cargo build Tauri binary
bun run build:all        # Full production build

bun run lint             # ESLint + cargo clippy
bun run lint:frontend    # ESLint on src/**/*.{ts,tsx}
bun run lint:backend     # cargo clippy -D warnings

bun run format           # Prettier + cargo fmt
bun run format:check     # Dry-run format validation
bun run typecheck        # tsc --noEmit

bun run sandbox:start    # Docker Compose up (pulls Ollama model)
bun run sandbox:stop     # docker compose down -v (destructive: wipes volume)

bun run test             # Vitest run (frontend tests only)
bun run test:watch       # Vitest watch mode
bun run test:coverage    # Vitest with coverage report
bun run test:backend          # Cargo test (Rust backend tests)
bun run test:backend:coverage # Cargo test + llvm-cov, enforces 100% line coverage (mirrors CI)
bun run test:all              # Both Vitest and Cargo test

bun run validate-build   # All gates: lint + format + typecheck + build
```

## Testing

Tests use **Vitest** for the frontend (React/TypeScript with React Testing Library + happy-dom) and **Cargo test** for the backend (Rust unit tests).

**100% code coverage is mandatory.** Any new or modified code — frontend or backend — must maintain 100% coverage across lines, functions, branches, and statements. PRs that drop below 100% coverage will not be merged.

- **Frontend:** Run `bun run test:coverage` and verify all metrics are 100%.
- **Backend:** Run `bun run test:backend:coverage` to enforce 100% line coverage (identical to what CI runs). Functions excluded from coverage with `#[cfg_attr(coverage_nightly, coverage(off))]` must be thin wrappers (Tauri commands, filesystem I/O) whose logic is tested through the functions they delegate to.

## Architecture

Thuki is a macOS-only desktop app, a floating AI secretary activated by double-tapping the Control key. It is a **Tauri v2** app (Rust backend + React/TypeScript frontend) that interfaces with a locally running **Ollama** instance at `http://127.0.0.1:11434`.

### Frontend (`src/`)

The UI morphs between two states: a compact spotlight-style input bar → an expanded chat window. This morphing is driven by Framer Motion and a single `isChatMode` boolean in `App.tsx`.

- **`App.tsx`** — orchestrates all state: messages, streaming, window resizing via ResizeObserver + Tauri `setSize()`
- **`hooks/useOllama.ts`** — Tauri Channel-based streaming hook; emits `Token`, `Done`, `Cancelled`, `Error` variants
- **`view/ConversationView.tsx`** — smart auto-scroll (pins to bottom unless user scrolls up)
- **`view/AskBarView.tsx`** — auto-expanding textarea (max 144px), morphs logo size, renders slash command tab-completion suggestions
- **`components/ChatBubble.tsx`** — markdown rendering via Streamdown (rehype-sanitize for XSS protection)
- **`config/commands.ts`** — slash command registry: defines supported commands and the `SCREEN_CAPTURE_PLACEHOLDER` sentinel used to show a loading tile in chat while a `/screen` capture is in flight

### Backend (`src-tauri/src/`)

- **`lib.rs`** — app setup: converts window to NSPanel (fullscreen overlay), registers tray, spawns hotkey listener, intercepts close events (hides instead of quits)
- **`commands.rs`** — `ask_ollama` Tauri command: streams newline-delimited JSON from Ollama, sends chunks via Tauri Channel
- **`screenshot.rs`** — `capture_full_screen_command` Tauri command: uses CoreGraphics FFI (`CGWindowListCreateImage`) to capture all displays excluding Thuki's own windows, writes a JPEG to a temp dir, and returns the path
- **`activator.rs`** — Core Graphics event tap watching for double-tap Control key (400ms window, 600ms cooldown). The tap MUST use `CGEventTapLocation::HID` and `CGEventTapOptions::Default` — see the critical constraint note in "Key Design Constraints" below.

### Sandbox (`sandbox/`)

Docker Compose runs Ollama in a hardened container: `cap_drop: ALL`, `no-new-privileges`, read-only model volume, localhost-only port binding (`127.0.0.1:11434`). Two services: `sandbox-init` (one-shot model pull) and `sandbox-server` (long-running daemon). `sandbox:stop` uses `down -v` which wipes the volume.

### IPC Pattern

Frontend calls Tauri commands via `@tauri-apps/api/core`. Streaming uses Tauri's **Channel API** — the Rust side sends typed `StreamChunk` enum variants, the hook accumulates tokens into React state.

### Window Lifecycle

- App starts hidden; hotkey or tray menu shows it
- Window close button hides (not quits); quit only from tray
- `ActivationPolicy::Accessory` hides Dock icon
- `macOSPrivateApi: true` enables NSPanel for fullscreen-app overlay

## Workflow

**Always use git worktrees for development work.** Before starting any feature, bugfix, or non-trivial change, create an isolated git worktree. This keeps the main working directory clean and allows parallel work without branch-switching conflicts.

### Git Worktree Requirements

1. **Never commit to main from a worktree.** All work must remain isolated in the worktree branch until explicitly tested and approved.
2. **Only merge to main after user sign-off.** User must confirm the fix/feature works before any changes land on main.
3. **Clean up on completion.** After work is approved and merged to main (or if abandoned), remove the worktree to keep the workspace tidy.
4. **Test in worktree first.** Verify all tests pass (100% coverage), build succeeds, and linting/formatting is clean before requesting approval.

## Post-Change Validation

After making any code changes and before ending your response, you must:

1. Run `bun run test` — all tests must pass
2. Run `bun run validate-build` — must complete with **zero warnings and zero errors**

Do not consider the task done if either step produces any warnings or errors. Fix all issues first.

## Superpowers Artifacts

Never commit files generated by superpowers skills (design specs, implementation plans, brainstorming docs). These live under `docs/superpowers/` which is gitignored. Do not stage or commit anything under that path.

## Key Design Constraints

- **macOS only** — uses NSPanel, Core Graphics event taps, macOS Control key
- **Privacy-first** — Ollama runs locally; Docker sandbox drops all capabilities and isolates network
- **Three permissions required** — Accessibility (CGEventTap creation), Input Monitoring (cross-app key delivery), Screen Recording (/screen command)

### CGEventTap configuration — DO NOT CHANGE these two settings

The hotkey listener in `activator.rs` requires **both** of the following settings to work correctly across all apps. Either one alone is insufficient; changing either one will silently break cross-app hotkey detection.

**`CGEventTapLocation::HID`** — must be HID level, never `Session` or `AnnotatedSession`.

Session-level taps (`kCGSessionEventTap`) sit above the window server routing layer. Since macOS 15 Sequoia, macOS applies focus-based filtering at that layer: a Session-level tap only receives events while the tap's own process (or its launch-parent terminal) has focus. Switching to any other app silently stops all event delivery. HID-level taps receive events before they reach the window server, bypassing this filtering entirely. This is what Karabiner-Elements, BetterTouchTool, and every other reliable system-wide key interceptor uses.

**`CGEventTapOptions::Default`** — must be the default (active) tap, never `ListenOnly`.

`ListenOnly` taps are disabled by macOS secure input mode. Secure input activates whenever a password field is focused, when iTerm's "Secure Keyboard Entry" is enabled, or when certain other security contexts are active. When the tap is disabled, macOS sends `TapDisabledByUserInput` and stops delivering events. Active (`Default`) taps are not subject to this restriction. We still return `CallbackResult::Keep` in the callback so no events are blocked or modified — the tap is passive in practice even though it is registered as active.
