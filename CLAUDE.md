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
bun run test:backend     # Cargo test (Rust backend tests)
bun run test:all         # Both Vitest and Cargo test

bun run validate-build   # All gates: lint + format + typecheck + build
```

## Git Configuration

All commits must be signed with your name and email. The repository is configured to automatically add your signature.

```bash
# One-time setup (if not already done)
git config user.name "Logan Nguyen"
git config user.email "lg.131.dev@gmail.com"
```

Every commit will automatically include:
```
Signed-off-by: Logan Nguyen <lg.131.dev@gmail.com>
```

The commit-msg hook in `.git/hooks/` handles this automatically. If you need to sign manually, use:
```bash
git commit -s  # -s flag adds Signed-off-by automatically
```

## Testing

Tests use **Vitest** for the frontend (React/TypeScript with React Testing Library + happy-dom) and **Cargo test** for the backend (Rust unit tests). Target 100% code coverage. Superpowers-generated documentation (specs, plans) should not be committed to PRs — only the test code and infrastructure.

## Architecture

Thuki is a macOS-only desktop app — a floating AI secretary activated by double-tapping the Command key. It is a **Tauri v2** app (Rust backend + React/TypeScript frontend) that interfaces with a locally running **Ollama** instance at `http://127.0.0.1:11434`.

### Frontend (`src/`)

The UI morphs between two states: a compact spotlight-style input bar → an expanded chat window. This morphing is driven by Framer Motion and a single `isChatMode` boolean in `App.tsx`.

- **`App.tsx`** — orchestrates all state: messages, streaming, window resizing via ResizeObserver + Tauri `setSize()`
- **`hooks/useOllama.ts`** — Tauri Channel-based streaming hook; emits `Token`, `Done`, `Error` variants
- **`view/ConversationView.tsx`** — smart auto-scroll (pins to bottom unless user scrolls up)
- **`view/AskBarView.tsx`** — auto-expanding textarea (max 144px), morphs logo size
- **`components/ChatBubble.tsx`** — markdown rendering with DOMPurify sanitization

### Backend (`src-tauri/src/`)

- **`lib.rs`** — app setup: converts window to NSPanel (fullscreen overlay), registers tray, spawns hotkey listener, intercepts close events (hides instead of quits)
- **`commands.rs`** — `ask_ollama` Tauri command: streams newline-delimited JSON from Ollama, sends chunks via Tauri Channel
- **`activator.rs`** — Core Graphics event tap watching for double-tap Command key (400ms window, 600ms cooldown); prompts for Accessibility permission, retries up to 6×

### Sandbox (`sandbox/`)

Docker Compose runs Ollama in a hardened container: `cap_drop: ALL`, `no-new-privileges`, read-only model volume, internal-only network. Two services: `sandbox-init` (one-shot model pull) and `sandbox-server` (long-running daemon). `sandbox:stop` uses `down -v` which wipes the volume.

### IPC Pattern

Frontend calls Tauri commands via `@tauri-apps/api/core`. Streaming uses Tauri's **Channel API** — the Rust side sends typed `StreamChunk` enum variants, the hook accumulates tokens into React state.

### Window Lifecycle

- App starts hidden; hotkey or tray menu shows it
- Window close button hides (not quits); quit only from tray
- `ActivationPolicy::Accessory` hides Dock icon
- `macOSPrivateApi: true` enables NSPanel for fullscreen-app overlay

## Key Design Constraints

- **macOS only** — uses NSPanel, Core Graphics event taps, macOS Command key
- **Privacy-first** — Ollama runs locally; Docker sandbox drops all capabilities and isolates network
- **Accessibility permission required** — hotkey listener uses a CGEventTap at session level
