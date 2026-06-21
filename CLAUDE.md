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

bun run engine:ensure    # Fetch + verify + re-sign the pinned llama-server sidecar (auto-runs before dev/build)

bun run search-box:start # Docker Compose up for the /search services (SearXNG + reader)
bun run search-box:stop  # docker compose down for the /search services

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

### Slash commands

User-facing reference for all commands lives in `docs/commands.md`. **Any new slash command must go through the same unified dispatch flow as the existing ones in `src/App.tsx`** (shared pre-flight in `handleSubmit`, then a command-specific stage-2 handler). Do not add a bespoke submit path; extend the existing dispatch instead. This keeps gating, deferral, capability checks, and cancellation behavior consistent across every command.

### Backend (`src-tauri/src/`)

- **`lib.rs`**: app setup: loads `AppConfig` via `config::load`, converts window to NSPanel (fullscreen overlay), registers tray, spawns hotkey listener, spawns the engine runner actor, intercepts close events (hides instead of quits), and on `RunEvent::Exit` kills the engine sidecar and awaits its confirmed exit so no orphan `llama-server` survives quit
- **`config/`**: typed TOML-backed application configuration. Loaded once at startup from `~/Library/Application Support/com.quietnode.thuki/config.toml` (seeded with defaults on first run), installed as Tauri managed state, exposed to the frontend via the `get_config` command. Every subsystem that needs model, prompt, window, activation, or quote values reads from `State<AppConfig>`. The `[inference]` section holds `active_provider`, `num_ctx`, `keep_warm_inactivity_minutes` (unified residency knob governing both local providers: the built-in engine's idle-unload timer and Ollama's `keep_alive`; not applicable to OpenAI), and the typed providers list (`[[inference.providers]]`, each `{id, kind, label, base_url, model, vision}`; `kind` is `builtin`, `ollama`, or `openai`, anything else is dropped on load). Fresh installs default `active_provider` to `builtin`; the loader pins any pre-providers config (no `[[inference.providers]]` array) to `ollama`, because no working built-in provider existed when that file was written. The loader also migrates a legacy flat `ollama_url` onto a synthesized Ollama provider, and `config/migrate.rs` folds the legacy SQLite `active_model` onto the active provider when it is Ollama-kind. See `docs/configurations.md` for the user-facing schema.
- **`commands.rs`**: `ask_model` Tauri command: routes by the active provider's kind. `builtin` resolves the installed model from the manifest, ensures the sidecar is loaded via the engine runner, and streams OpenAI-compatible `/v1/chat/completions` SSE through `openai.rs` (`V1Flavor::Builtin`); `ollama` streams the native `/api/chat` newline-delimited JSON; `openai` streams `/v1` SSE against the provider's `base_url` (`V1Flavor::Remote`). All paths emit the same `StreamChunk` contract via Tauri Channel and read the active provider, the resolved system prompt, and the in-memory `ActiveModelState` from managed state.
- **`keychain.rs`**: write-only storage for `openai`-provider API keys in the macOS Keychain via the `keyring` crate. The Keychain is the only place keys ever live: they are never written to the TOML config and never returned to the frontend (only existence is queryable via `has_provider_api_key`); the `SecretStore` trait decouples callers from the real Keychain for tests.
- **`screenshot.rs`** — `capture_full_screen_command` Tauri command: uses CoreGraphics FFI (`CGWindowListCreateImage`) to capture all displays excluding Thuki's own windows, writes a JPEG to a temp dir, and returns the path
- **`activator.rs`** — Core Graphics event tap watching for double-tap Control key (400 ms window, 600 ms cooldown; timing is a compiled constant, not yet exposed through `AppConfig` because the event-tap callback runs in a thread that cannot trivially read Tauri managed state). The tap MUST use `CGEventTapLocation::HID` and `CGEventTapOptions::Default` — see the critical constraint note in "Key Design Constraints" below.

### Built-in engine (`src-tauri/src/engine/`)

Thuki bundles llama.cpp's `llama-server` and manages its lifecycle: at most one engine process exists, never two models are resident, and a model or context-size switch always kills the old process and waits for a confirmed exit before spawning the new one.

- **`state.rs`**: pure, side-effect-free residency state machine: `Stopped`, `Starting(Target)`, `Loaded { target, port }`, `Stopping { next }`, `Failed(String)`. A `Target` is `{model_path, mmproj_path, num_ctx}`; two targets are interchangeable only when **every** field is equal, so a `num_ctx` change is a different target and forces a restart exactly like a model switch (the context size is fixed at `llama-server` startup).
- **`runner.rs`**: async actor that owns the live child process. Commands (`Ensure`, `Touch`, `SetIdleMinutes`, `Unload`, `Shutdown`) arrive on a bounded mpsc channel (`ENGINE_COMMAND_QUEUE_CAPACITY`); every transition is published on a `watch` channel for the frontend status. Startup readiness is a `/health` poll loop governed by the `ENGINE_HEALTH_*` constants; `idle_unload_minutes` of inactivity (checked every `ENGINE_IDLE_CHECK_INTERVAL_SECS`) stops the engine to free RAM.
- **`process.rs`**: the real `EngineProcess` backed by `tokio::process` + reqwest. Spawn line: `-m <model> [--mmproj <p>] --ctx-size <n> --host 127.0.0.1 --port <p> --no-webui`. The bind is localhost-only and the web UI is disabled; do not change either.

Sidecar constraints: the binary ships through tauri.conf `externalBin` (`binaries/llama-server`) and its dylib closure is bundled via the macOS `frameworks` list, resolved at runtime through the `@loader_path/../Frameworks` rpath that `scripts/ensure-llama-server.ts` adds (the script fetches the pinned llama.cpp release, verifies its sha256, prunes the dylib closure, and ad-hoc re-signs everything; it auto-runs in front of `dev` and the build scripts). The process is spawned with `tokio::process`, not Tauri's shell plugin, so the runner owns kill/wait directly; `lib.rs` shuts the sidecar down on app quit (kill-on-quit, see above).

### Model library (`src-tauri/src/models/`)

- **`mod.rs`**: active-model state (`ActiveModelState`, picker plumbing, persistence onto the active provider's `model` field) plus the public download/cancel API with a single-download-at-a-time slot.
- **`registry.rs`**: curated starters in three tiers (Fast / Balanced / Smartest). Every entry pins a Hugging Face repo at an exact git revision and carries each blob's sha256, size, capability flags (vision/thinking, mmproj companion), and license note.
- **`download.rs`**: resumable downloader: streams from Hugging Face into blob-store partials, resumes via HTTP `Range`, emits `DownloadEvent`s throttled by `DOWNLOAD_PROGRESS_MIN_INTERVAL_MS`, and verifies sha256 on completion. The hash check is an integrity check only (truncation, bit rot, resume corruption), never a supply-chain/provenance control; provenance comes from the pinned repo revisions.
- **`storage.rs`**: content-addressed blob store: `root/tmp/<sha256>.partial` during download, streaming SHA-256 verify, then atomic rename into `root/blobs/<sha256>`.
- **`manifest.rs`**: CRUD over the `installed_models` SQLite table; row id is `"<repo>:<file_name>"`, content addresses shared across rows (two models can reference the same mmproj blob).

### Sandbox (`sandbox/`)

`sandbox/search-box/` runs the SearXNG + reader services behind `/search` as a Docker Compose stack.
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
- **`config/schema.rs`** — typed TOML shape (`AppConfig` + per-section structs like `SearchSection`). Each section has a manual `Default` impl that pulls from `defaults.rs`. Use `#[serde(default)]` on every section so partial files load cleanly.
- **`config/loader.rs`** — read → parse → resolve. `resolve` empties strings to defaults, clamps numerics via `clamp_u32`/`clamp_u64`/`clamp_f64`, composes the prompt appendix, and enforces cross-field invariants (e.g. `reader_batch_timeout_s > reader_per_url_timeout_s`).
- **`config/writer.rs`** — atomic write used to seed the file on first run.
- **`AppConfig` is installed as Tauri managed state** once at startup in `lib.rs`. Subsystems that need config read from `State<AppConfig>` and nowhere else.

### Subsystem projections

Some subsystems do not want a transitive dependency on the whole TOML schema. They take a flat projection instead. The pattern: a `Subsystem RuntimeConfig` struct with a `from_app_config(&AppConfig) -> Self` constructor and a `Default` impl that reads `defaults::*`. See `src-tauri/src/search/config.rs` (`SearchRuntimeConfig`) for the canonical example. This isolates schema changes to one adapter file and keeps the subsystem's tests free of `AppConfig` setup.

### Adding a new user-tunable field (checklist)

1. Add `DEFAULT_<NAME>` in `config/defaults.rs`. For numerics, also add `BOUNDS_<NAME>: (T, T)`.
2. Add the field to the matching section struct in `config/schema.rs` and to its `Default` impl. Use `pub` and a doc comment that explains the tunable's user-facing meaning, not its implementation.
3. Add a `clamp_*` (or string-empty fallback) call in `loader::resolve`.
4. If a subsystem uses a `RuntimeConfig` projection, add the field there and to `from_app_config` + `Default` + the field-by-field assertion test.
5. Cover it in `config/tests.rs`: schema default matches `DEFAULT_*`, out-of-bounds → default, in-bounds preserved, TOML round-trip carries the field.
6. Update `docs/configurations.md`: add a row to the matching domain table, update the example TOML at the top of the file. For numeric fields, include a "Raise for X; lower for Y" trade-off in the description (see `[search]` rows for the tone).

### Adding a new baked-in constant

Same first step (`config/defaults.rs`), but no schema/loader changes. Reference it from the consuming module via `use crate::config::defaults::*`. Add a baked-in row to `docs/configurations.md` under the matching domain table with a clear "Why not tunable" rationale. Valid rationales: defense-in-depth bound on external/attacker-controlled data, prompt contract (constant referenced in a hardcoded LLM prompt), protocol cap imposed by an external service, hardware constant (key code), thread-safety blocker for plumbing user state.

### Bad-input behavior

The loader is forgiving and never crashes the app on user config:

- Missing file → defaults seeded and written. (Only fatal failure path is the seed write itself.)
- Missing fields/sections → `#[serde(default)]` fills from compiled defaults.
- Empty/whitespace strings → replaced with compiled defaults. Exception: `prompt.system` is governed by `prompt.system_customized`. When `system_customized = true` the stored value is a deliberate user override and is respected verbatim, including an empty value (which means "send no persona", composing only the slash-command appendix into `resolved_system`). When `system_customized = false` the stored `system` is not authoritative (it is only a cached copy of the default seeded at first run), so it is always replaced with the current `DEFAULT_SYSTEM_PROMPT_BASE`. This heals configs predating the Settings UI and propagates later edits of the built-in prompt to every non-customizing install.
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

## Post-Change Validation

After making any code changes and before ending your response, you must:

1. Run `bun run test:all:coverage` — frontend + backend tests must pass AND 100% coverage gate must hold
2. Run `bun run validate-build` — must complete with **zero warnings and zero errors**

Do not consider the task done if either step produces any warnings or errors. Fix all issues first.

## Superpowers Artifacts

Never commit files generated by superpowers skills (design specs, implementation plans, brainstorming docs). These live under `docs/superpowers/` which is gitignored. Do not stage or commit anything under that path.

## GStack Design Tooling Fallback

When invoking GStack design skills (`/design-shotgun`, `/design-html`, `/design-review`, etc.) inside Claude Code on this project: if the design CLI fails because no OpenAI API key is configured (e.g. `setup` not run, `OPENAI_API_KEY` unset, `~/.gstack/openai.json` missing), do not block the user with a setup prompt. Automatically fall back to hand-crafted HTML wireframes that use the real Thuki design tokens read directly from the source files (`src/view/onboarding/PermissionsStep.tsx`, `src/view/onboarding/IntroStep.tsx`, `src/components/`). These wireframes are strictly more accurate to the final UI than image generation because they use the exact CSS values rather than a model's interpretation of them.

Workflow:
1. Read the relevant source files to extract the actual design tokens (colors, spacing, fonts, border radii, gradients, shadows).
2. Write the wireframes as static HTML files in `~/.gstack/projects/quiet-node-thuki/designs/<screen-name>-<date>/` so they live alongside any future image-based mockups.
3. Open the wireframes in the browser via `open file://...` for review.
4. Only mention the missing API key as a one-line aside, not as a blocker. The user can opt back into image generation later.

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
