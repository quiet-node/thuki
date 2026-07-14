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

bun run engine:ensure    # Build + verify + sign the pinned llama-server sidecar from source (auto-runs before dev/build)

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

**Always run `bun run test:all:coverage` (never the bare `bun run test` / `bun run test:all`).** This single command runs both Vitest with coverage and the cargo llvm-cov gate that CI enforces. If it does not exit cleanly, the task is not done. Functions excluded from coverage with `#[cfg_attr(coverage_nightly, coverage(off))]` must be thin wrappers (Tauri commands, filesystem I/O) whose logic is tested through the functions they delegate to.

## Since v0.15

Since v0.15, Thuki ships its own inference engine and is local-first out of the box. This is the headline shift: where earlier versions talked only to a separately installed Ollama, Thuki now runs models itself with nothing to set up. This is the at-a-glance map of what changed; the engine and model-library internals are detailed in `docs/models-and-providers.md`.

- **Built-in engine is the default provider**: a bundled llama.cpp `llama-server` sidecar, fully managed by Thuki (spawned, health-checked, kept warm, and killed on quit). See `src-tauri/src/engine/` and `docs/models-and-providers.md`.
- **In-app model library**: download GGUF models from Hugging Face (curated Staff picks + raw Browse all) without a terminal, into a content-addressed blob store, resumable and SHA-256 verified. See `src-tauri/src/models/`.
- **Providers**: built-in (default) and Ollama for end users; an OpenAI-compatible `openai` kind exists in code but stays gated and is not exposed to end users. Switching providers frees the deactivated one's memory, so only one is resident at a time. See `src-tauri/src/config/` and `src-tauri/src/commands.rs`.
- **Keep Warm, RAM-fit hint, and reasoning via `/think`**: a residency knob that holds the model in memory between messages, a Comfortable/Tight/Heavy memory-fit verdict per model, and opt-in per-message reasoning.
- **curl \| sh install**: a Gatekeeper-friendly install path (the downloaded archive carries no quarantine attribute), with the release artifact RSA-4096 signed and verified by stock `openssl`.

## Architecture

Thuki is a macOS-only desktop app, a floating AI secretary activated by double-tapping the Control key. Project homepage: [thuki.app](https://www.thuki.app/). It is a **Tauri v2** app (Rust backend + React/TypeScript frontend) that ships its own inference engine: a bundled **llama.cpp** `llama-server` sidecar spawned and supervised by the backend (the default provider on fresh installs). It can instead talk to a locally running **Ollama** instance (default `http://127.0.0.1:11434`) or any OpenAI-compatible `/v1` server.

### Frontend (`src/`)

The UI morphs between two states: a compact spotlight-style input bar → an expanded chat window. This morphing is driven by Framer Motion and a single `isChatMode` boolean in `App.tsx`.

- **`App.tsx`** — orchestrates all state: messages, streaming, window resizing via ResizeObserver + Tauri `setSize()`
- **`hooks/useModel.ts`** — Tauri Channel-based streaming hook (`useModel`); emits `Token`, `Done`, `Cancelled`, `Error` variants
- **`view/ConversationView.tsx`** — smart auto-scroll (pins to bottom unless user scrolls up)
- **`view/AskBarView.tsx`** — auto-expanding textarea (max 144px), morphs logo size, renders slash command tab-completion suggestions
- **`components/ChatBubble.tsx`** — markdown rendering via Streamdown (rehype-sanitize for XSS protection)
- **`config/commands.ts`** — slash command registry: defines supported commands and the `SCREEN_CAPTURE_PLACEHOLDER` sentinel used to show a loading tile in chat while a `/screen` capture is in flight
- **`components/CommandSuggestion.tsx`** — slash command autocomplete popover. Contains `iconForTrigger()`, a switch statement mapping trigger strings to inline SVG constants. **Every new slash command needs a dedicated case here.** Without it, the command falls through to the default, which returns `SCREEN_ICON` (the monitor icon). Steps: (1) add a hoisted `const FOO_ICON = (<svg .../>)` constant, (2) add `case '/foo': return FOO_ICON;` to `iconForTrigger()`.
- **`components/PointingWiggle.tsx`** (+ `PointingWiggle.module.css`) — **the only** UI for “click something, then point at a target” (Settings deep-links, in-app “look here”). Hand-drawn primary squiggle: draw → settle → three soft breaths → fade (`POINTING_WIGGLE_MS` = 7200). Use `PointingWiggle` as a label accessory or `PointingLabel` around the target text. **Do not invent rings, chips, or alternate pointer chrome.** Keep deep-link **routing** (which command/event opens which tab) unchanged; only add/remove this UI highlight on the existing landing control. Current call site: Behavior → Auto search (version announcement Settings CTA). Reuse the same component when adding new point-to-target moments.
- **`components/VersionAnnouncement.tsx`** + **`config/versionAnnouncements.ts`** — reusable one-shot version/feature announcement panel (ask-bar footer design D). Shell is content-agnostic (title, body, optional learn link, primary/secondary actions). Per-release copy lives in `versionAnnouncements.ts` (e.g. `V016_AUTO_SEARCH_ANNOUNCEMENT`). Host owns show/hide and dismiss persistence (today: `behavior.search_notice_acknowledged`). **Do not invent a second announcement chrome.**

### Slash commands

User-facing reference for all commands lives in `docs/commands.md`. **Any new slash command must go through the same unified dispatch flow as the existing ones in `src/App.tsx`** (shared pre-flight in `handleSubmit`, then a command-specific stage-2 handler). Do not add a bespoke submit path; extend the existing dispatch instead. This keeps gating, deferral, capability checks, and cancellation behavior consistent across every command.

### Backend (`src-tauri/src/`)

- **`lib.rs`**: app setup: loads `AppConfig` via `config::load`, converts window to NSPanel (fullscreen overlay), registers tray, spawns hotkey listener, spawns the engine runner actor, intercepts close events (hides instead of quits), and on `RunEvent::Exit` kills the engine sidecar and awaits its confirmed exit so no orphan `llama-server` survives quit
- **`config/`**: typed TOML-backed configuration, loaded once at startup from `~/Library/Application Support/com.quietnode.thuki/config.toml` (seeded with defaults on first run), installed as Tauri managed state, and exposed to the frontend via `get_config`. Subsystems read model, prompt, window, activation, and quote values from `State<AppConfig>`. The `[inference]` section holds `active_provider` (defaults to `builtin`), `num_ctx`, `keep_warm_inactivity_minutes` (the unified residency knob for both local providers), and the typed providers list (`[[inference.providers]]`, each `{id, kind, label, base_url, model, vision}`; `kind` is `builtin`, `ollama`, or `openai`, anything else is dropped on load). The loader heals the built-in label to the current default and migrates older configs in place (a legacy `ollama_url` becomes an Ollama provider; `config/migrate.rs` folds a legacy SQLite `active_model` onto an Ollama-kind active provider). See `docs/configurations.md` for the user-facing schema.
- **`commands.rs`**: `ask_model` Tauri command: routes by the active provider's kind. `builtin` resolves the installed model from the manifest, ensures the sidecar is loaded via the engine runner, and streams OpenAI-compatible `/v1/chat/completions` SSE through `openai.rs` (`V1Flavor::Builtin`); `ollama` streams the native `/api/chat` newline-delimited JSON; `openai` streams `/v1` SSE against the provider's `base_url` (`V1Flavor::Remote`). All paths emit the same `StreamChunk` contract via Tauri Channel and read the active provider, the resolved system prompt, and the in-memory `ActiveModelState` from managed state.
- **`keychain.rs`**: write-only storage for `openai`-provider API keys in the macOS Keychain via the `keyring` crate. The Keychain is the only place keys ever live: they are never written to the TOML config and never returned to the frontend (only existence is queryable via `has_provider_api_key`); the `SecretStore` trait decouples callers from the real Keychain for tests.
- **`screenshot.rs`** — `capture_full_screen_command` Tauri command: uses CoreGraphics FFI (`CGWindowListCreateImage`) to capture all displays excluding Thuki's own windows, writes a JPEG to a temp dir, and returns the path
- **`activator.rs`** — Core Graphics event tap watching for double-tap Control key (400 ms window, 600 ms cooldown; timing is a compiled constant, not yet exposed through `AppConfig` because the event-tap callback runs in a thread that cannot trivially read Tauri managed state). The tap MUST use `CGEventTapLocation::HID` and `CGEventTapOptions::Default` — see the critical constraint note in "Key Design Constraints" below.

### IPC Pattern

Frontend calls Tauri commands via `@tauri-apps/api/core`. Streaming uses Tauri's **Channel API** — the Rust side sends typed `StreamChunk` enum variants, the hook accumulates tokens into React state.

### Window Lifecycle

- App starts hidden; hotkey or tray menu shows it
- Window close button hides (not quits); quit only from tray
- `ActivationPolicy::Accessory` hides Dock icon
- `macOSPrivateApi: true` enables NSPanel for fullscreen-app overlay

## Configuration System

Thuki has a single, typed configuration system rooted in `src-tauri/src/config/`. Read `docs/configurations.md` for the user-facing schema. The rules below tell you how the pieces fit so you can extend it without drift.

### Single source of truth

Every default value and every numeric bound lives in **`config/defaults.rs`** as `DEFAULT_*` and `BOUNDS_*` consts. No subsystem owns its own copy of a default. If you find one (e.g. a hardcoded number in a search/image/UI module), move it here and reference it via `use crate::config::defaults::*`. This applies to BOTH user-tunable defaults AND baked-in pipeline constants.

### Layered structure

- **`config/defaults.rs`** — every constant Thuki uses. Tunable defaults, hard bounds, and baked-in pipeline constants all live here.
- **`config/schema.rs`** — typed TOML shape (`AppConfig` + per-section structs like `WindowSection`). Each section has a manual `Default` impl that pulls from `defaults.rs`. Use `#[serde(default)]` on every section so partial files load cleanly.
- **`config/loader.rs`** — read → parse → resolve. `resolve` empties strings to defaults, clamps numerics via `clamp_u32`/`clamp_u64`/`clamp_f64`, composes the prompt appendix, and applies any cross-field corrections a section needs. Unknown keys and whole unknown sections are silently ignored (no `deny_unknown_fields`), so a config written by an older build still loads clean.
- **`config/writer.rs`** — atomic write used to seed the file on first run.
- **`AppConfig` is installed as Tauri managed state** once at startup in `lib.rs`. Subsystems that need config read from `State<AppConfig>` and nowhere else.

### Subsystem projections

Some subsystems do not want a transitive dependency on the whole TOML schema. They take a flat projection instead. The pattern: a `Subsystem RuntimeConfig` struct with a `from_app_config(&AppConfig) -> Self` constructor and a `Default` impl that reads `defaults::*`. This isolates schema changes to one adapter file and keeps the subsystem's tests free of `AppConfig` setup.

### Adding a new user-tunable field (checklist)

1. Add `DEFAULT_<NAME>` in `config/defaults.rs`. For numerics, also add `BOUNDS_<NAME>: (T, T)`.
2. Add the field to the matching section struct in `config/schema.rs` and to its `Default` impl. Use `pub` and a doc comment that explains the tunable's user-facing meaning, not its implementation.
3. Add a `clamp_*` (or string-empty fallback) call in `loader::resolve`.
4. If a subsystem uses a `RuntimeConfig` projection, add the field there and to `from_app_config` + `Default` + the field-by-field assertion test.
5. Cover it in `config/tests.rs`: schema default matches `DEFAULT_*`, out-of-bounds → default, in-bounds preserved, TOML round-trip carries the field.
6. Update `docs/configurations.md`: add a row to the matching domain table, update the example TOML at the top of the file. For numeric fields, include a "Raise for X; lower for Y" trade-off in the description (see `[quote]` rows for the tone).

### Adding a new baked-in constant

Same first step (`config/defaults.rs`), but no schema/loader changes. Reference it from the consuming module via `use crate::config::defaults::*`. Add a baked-in row to `docs/configurations.md` under the matching domain table with a clear "Why not tunable" rationale. Valid rationales: defense-in-depth bound on external/attacker-controlled data, prompt contract (constant referenced in a hardcoded LLM prompt), protocol cap imposed by an external service, hardware constant (key code), thread-safety blocker for plumbing user state.

### Bad-input behavior

The loader is forgiving and never crashes the app on user config:

- Missing file → defaults seeded and written. (Only fatal failure path is the seed write itself.)
- Missing fields/sections → `#[serde(default)]` fills from compiled defaults.
- Empty/whitespace strings → compiled defaults, except `prompt.system` (governed by `prompt.system_customized`): when `true`, the stored value is respected verbatim, including an empty value (meaning "send no persona", just the slash-command appendix); when `false`, it is treated as a cached default and replaced with the current `DEFAULT_SYSTEM_PROMPT_BASE`, so built-in-prompt edits reach every non-customizing install.
- Out-of-bounds numerics → reset to default with a stderr warning.
- Unparseable TOML → file renamed `config.toml.corrupt-<unix_ts>` and a fresh defaults file written.

When extending the system, preserve this contract: **never panic on user input**.

## Workflow

**Always use git worktrees for development work.** Before starting any feature, bugfix, or non-trivial change, create an isolated git worktree. This keeps the main working directory clean and allows parallel work without branch-switching conflicts.

### Git Worktree Requirements

1. **Never commit to main from a worktree.** All work must remain isolated in the worktree branch until explicitly tested and approved.
2. **Only merge to main after user sign-off.** User must confirm the fix/feature works before any changes land on main.
3. **Clean up on completion.** After work is approved and merged to main (or if abandoned), remove the worktree to keep the workspace tidy.
4. **Test in worktree first.** Verify all tests pass (100% coverage), build succeeds, and linting/formatting is clean before requesting approval.

### Pull Request Descriptions

Keep PR descriptions compact and to the point, never an essay. Cover only what a reviewer needs to understand the change. Use simple language, action verbs, and plain phrasing so anyone can grasp it on a first read.

Structure:

1. **Description** — a short paragraph (1 to 3 sentences) stating what the PR does and why.
2. **Key changes** — a compact bullet list of the concrete changes.
3. Add further sections only when they add value (for example testing notes), and keep them short.

## Post-Change Validation

After making any code changes and before ending your response, you must:

1. Run `bun run test:all:coverage` — frontend + backend tests must pass AND 100% coverage gate must hold
2. Run `bun run validate-build` — must complete with **zero warnings and zero errors**

Do not consider the task done if either step produces any warnings or errors. Fix all issues first.

## Superpowers Artifacts

Never commit files generated by superpowers skills (design specs, implementation plans, brainstorming docs). These live under `docs/superpowers/` which is gitignored. Do not stage or commit anything under that path.

## GStack Design Tooling Fallback

When a GStack design skill (`/design-shotgun`, `/design-html`, etc.) fails because no OpenAI API key is configured, do not block on a setup prompt. Fall back to hand-crafted HTML wireframes built from the real Thuki design tokens read directly from source (`src/view/onboarding/*`, `src/components/`); these beat image generation because they use the exact CSS values. Write them under `~/.gstack/projects/quiet-node-thuki/designs/<screen>-<date>/`, open with `open file://...`, and mention the missing key only as a one-line aside.

## Key Design Constraints

- **macOS only** — uses NSPanel, Core Graphics event taps, macOS Control key
- **Privacy-first**: all inference is local (bundled llama.cpp engine by default; optional local Ollama or OpenAI-compatible servers)
- **Two permissions required** — Accessibility (CGEventTap creation), Screen Recording (/screen command)

### CGEventTap configuration — DO NOT CHANGE these two settings

The hotkey listener in `activator.rs` requires **both** of the following settings to work correctly across all apps. Either one alone is insufficient; changing either one will silently break cross-app hotkey detection.

**`CGEventTapLocation::HID`** — must be HID level, never `Session` or `AnnotatedSession`.

Session-level taps (`kCGSessionEventTap`) sit above the window server routing layer. Since macOS 15 Sequoia, macOS applies focus-based filtering at that layer: a Session-level tap only receives events while the tap's own process (or its launch-parent terminal) has focus. Switching to any other app silently stops all event delivery. HID-level taps receive events before they reach the window server, bypassing this filtering entirely. This is what Karabiner-Elements, BetterTouchTool, and every other reliable system-wide key interceptor uses.

**`CGEventTapOptions::Default`** — must be the default (active) tap, never `ListenOnly`.

`ListenOnly` taps are disabled by macOS secure input mode. Secure input activates whenever a password field is focused, when iTerm's "Secure Keyboard Entry" is enabled, or when certain other security contexts are active. When the tap is disabled, macOS sends `TapDisabledByUserInput` and stops delivering events. Active (`Default`) taps are not subject to this restriction. We still return `CallbackResult::Keep` in the callback so no events are blocked or modified — the tap is passive in practice even though it is registered as active.
