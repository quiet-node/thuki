# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- **Unified trace recorder** (`/feat/conversation-tracing`). The forensic JSONL recorder that shipped in PR #126 for the search pipeline now also captures every chat turn: user messages, assistant streaming tokens, thinking output, screen captures, and conversation lifecycle events (`conversation_start`, `conversation_end`). Toggle via `[debug] trace_enabled = true` (or the Settings panel "Trace recording" toggle in Web → Diagnostics). Output: per-conversation JSONL files under `~/Library/Application Support/com.quietnode.thuki/traces/{chat,search}/<conversation_id>.jsonl`. Off by default. Intended for power-user analysis: point Claude Code, jq, or a Python notebook at the folder to study patterns and improve the system prompt. See `docs/configurations.md` `[debug]` for the full schema, file layout, and late-event tolerance contract.

### Changed

- **BREAKING**: Renamed the `[debug] search_trace_enabled` config field to `trace_enabled` (the field now governs both chat and search domains, not just search). Existing `search_trace_enabled = true` configs continue to work via `serde(alias)`; the first launch on the new build silently rewrites the field name to `trace_enabled` if anything causes the loader to serialize back to disk. The trace file layout also moved from `traces/<turn-id>.jsonl` (one per search turn) to `traces/{chat,search}/<conversation_id>.jsonl` (one per conversation, per domain). Schema version bumped from `1` to `2`; v2 lines additionally carry top-level `domain` and `conversation_id` fields. Consumers that grep / jq the trace files must update.
- The `ask_ollama`, `search_pipeline`, and `capture_full_screen_command` Tauri commands now require a `conversationId: String` argument (and `ask_ollama` additionally requires `isFirstTurn: bool` and `slashCommand: Option<String>`). The frontend's `useOllama` hook generates a stable trace id per session and threads it transparently. External callers that invoked these commands directly must update their `invoke()` calls. A new fire-and-forget `record_conversation_end` command lets the frontend signal end-of-conversation (used by `useOllama.reset()` and `useOllama.loadMessages()`) so the chat-domain trace file gets a clean closing line.
- **BREAKING**: Renamed the `[model]` section in `config.toml` to `[inference]`. The section still contains a single field, `ollama_url`, but the name now reflects what it actually configures (the inference daemon endpoint, not a model). There is no backward-compatibility shim: if you had a custom `[model]` section, rename it to `[inference]` after upgrading.
- Active model selection is now strictly Option-typed end to end. Ollama's `/api/tags` is the single source of truth: when nothing is installed and nothing is persisted, Thuki refuses to dispatch requests and surfaces a "Pick a model" prompt instead of falling back to a hardcoded slug. The previous `DEFAULT_MODEL_NAME` constant has been removed.

## [0.7.1](https://github.com/quiet-node/thuki/compare/v0.7.0...v0.7.1) (2026-05-04)


### Bug Fixes

* **settings:** repair keep-warm minutes input UX ([#127](https://github.com/quiet-node/thuki/issues/127)) ([38b506c](https://github.com/quiet-node/thuki/commit/38b506cdd817b728387bf0c864c15e23eb62844b))

## [0.7.0](https://github.com/quiet-node/thuki/compare/v0.6.1...v0.7.0) (2026-05-04)


### Features

* add utility slash commands ([#93](https://github.com/quiet-node/thuki/issues/93)) ([98a3a19](https://github.com/quiet-node/thuki/commit/98a3a196710edfbd99c9860753fea5cbfaf9c28b))
* **ci:** add floating nightly release workflow ([#109](https://github.com/quiet-node/thuki/issues/109)) ([c213235](https://github.com/quiet-node/thuki/commit/c2132358da02428d77b43a4e288f4dc987782ca2))
* **config:** make max_images user-tunable with a cap of 20 ([#121](https://github.com/quiet-node/thuki/issues/121)) ([4e1b3af](https://github.com/quiet-node/thuki/commit/4e1b3afbbf3c2caa116e84bfdedd5cec941709a6))
* **config:** migrate runtime configuration from env vars to TOML ([#102](https://github.com/quiet-node/thuki/issues/102)) ([20abeb0](https://github.com/quiet-node/thuki/commit/20abeb025655159f9ad5bcc4287ea8f76eda6026))
* **config:** user-tunable context window with log-scale slider ([#120](https://github.com/quiet-node/thuki/issues/120)) ([1c18ddf](https://github.com/quiet-node/thuki/commit/1c18ddf56ea50607fe034945f38d79edd123d885))
* **continuity:** cross-model history sanitization and capability-aware filtering ([#107](https://github.com/quiet-node/thuki/issues/107)) ([c976d63](https://github.com/quiet-node/thuki/commit/c976d63a6b8b1f9ac171fd988ec54260dba3beae))
* in-app model picker with hardened selection pipeline ([#103](https://github.com/quiet-node/thuki/issues/103)) ([d6cf4fb](https://github.com/quiet-node/thuki/commit/d6cf4fb576e72029834d53c12a844fed6a41a975))
* introduce agentic search pipeline with live trace streaming ([#100](https://github.com/quiet-node/thuki/issues/100)) ([445534f](https://github.com/quiet-node/thuki/commit/445534f0835ebe8b2e60e8d6a6f741b052534215))
* **model-picker:** add larger-models nudge hint ([#118](https://github.com/quiet-node/thuki/issues/118)) ([6c0df18](https://github.com/quiet-node/thuki/commit/6c0df189450ac1eb21dfe2d8d571c1ec9e48b8af))
* **search:** add forensic trace recorder ([#126](https://github.com/quiet-node/thuki/issues/126)) ([e1d5997](https://github.com/quiet-node/thuki/commit/e1d5997572150b1b8a77c1c0b4a50943656dddb1))
* sync slash command docs and prompt metadata ([#101](https://github.com/quiet-node/thuki/issues/101)) ([7501d60](https://github.com/quiet-node/thuki/commit/7501d601d5fe83e33778737a68a84b9fcb968e03))
* **tray:** left-click opens Thuki, right-click shows menu ([#123](https://github.com/quiet-node/thuki/issues/123)) ([81f133e](https://github.com/quiet-node/thuki/commit/81f133e1f2a8c04a151caefbaf8f673a53969284))
* **ui:** add tip bar with contextual usage tips ([#119](https://github.com/quiet-node/thuki/issues/119)) ([ed9b250](https://github.com/quiet-node/thuki/commit/ed9b2504c98fd95a90395c4fe398367872c8f15d))


### Bug Fixes

* **chat:** prevent source-row clicks from opening URL twice ([#104](https://github.com/quiet-node/thuki/issues/104)) ([e1d2cdf](https://github.com/quiet-node/thuki/commit/e1d2cdf85c2f81219784536779cd7048340df2fa))
* **ci:** set VITE_GIT_COMMIT_SHA on tauri build step not frontend step ([#111](https://github.com/quiet-node/thuki/issues/111)) ([ed80d15](https://github.com/quiet-node/thuki/commit/ed80d151f907313c44be6d92cf2017be3c78d802))
* **search:** correct Setup Guide anchor in sandbox-offline card ([#112](https://github.com/quiet-node/thuki/issues/112)) ([29f2c1f](https://github.com/quiet-node/thuki/commit/29f2c1f2af7e2c8631e40d336b8735e5c8acbdcd))
* **search:** harden judge fallback and config allowlist ([#125](https://github.com/quiet-node/thuki/issues/125)) ([cf82a95](https://github.com/quiet-node/thuki/commit/cf82a95f722573cd282a2ffec3c2e94e84e9ec12))
* **settings:** allow text selection in settings panel ([#122](https://github.com/quiet-node/thuki/issues/122)) ([5c552cb](https://github.com/quiet-node/thuki/commit/5c552cb9782636b359b0ee7d1c95de5b5bc83350))
* **settings:** eliminate Dock icon by converting settings window to NSPanel ([#117](https://github.com/quiet-node/thuki/issues/117)) ([217fa00](https://github.com/quiet-node/thuki/commit/217fa00ef4b570cadda33d44d44e2c3ef65fcedd))

## [0.6.1](https://github.com/quiet-node/thuki/compare/v0.6.0...v0.6.1) (2026-04-14)


### Bug Fixes

* intercept drops at root level and add max-images UX feedback ([#90](https://github.com/quiet-node/thuki/issues/90)) ([c304af8](https://github.com/quiet-node/thuki/commit/c304af8e1ffc32567228bd6910ecacdad1150991))

## [0.6.0](https://github.com/quiet-node/thuki/compare/v0.5.2...v0.6.0) (2026-04-14)


### Features

* add /think command with thinking mode UI ([#85](https://github.com/quiet-node/thuki/issues/85)) ([59f7333](https://github.com/quiet-node/thuki/commit/59f7333335a55a896209b5c7756368988b80cf49))

## [0.5.2](https://github.com/quiet-node/thuki/compare/v0.5.1...v0.5.2) (2026-04-12)


### Bug Fixes

* enlarge close button hit area to fix unreliable click ([#82](https://github.com/quiet-node/thuki/issues/82)) ([a829858](https://github.com/quiet-node/thuki/commit/a829858b8458e70fa704c0174e0589cdb4728feb))

## [0.5.1](https://github.com/quiet-node/thuki/compare/v0.5.0...v0.5.1) (2026-04-10)


### Bug Fixes

* cancel active streaming on overlay hide and app quit ([#73](https://github.com/quiet-node/thuki/issues/73)) ([077893a](https://github.com/quiet-node/thuki/commit/077893aa6252d8dbf967c82ffd1aa1e5af39b32c))
* preserve scroll position when streaming finishes ([#70](https://github.com/quiet-node/thuki/issues/70)) ([4254ea2](https://github.com/quiet-node/thuki/commit/4254ea20afa7a4341c87efc6ceeda59686bc35f7))
* replace anchor system with simple screen-bottom growth detection ([#74](https://github.com/quiet-node/thuki/issues/74)) ([d59119d](https://github.com/quiet-node/thuki/commit/d59119d1da2a47b80a3c0747ffea9d1d5d78df98))

## [0.5.0](https://github.com/quiet-node/thuki/compare/v0.4.0...v0.5.0) (2026-04-08)


### Features

* friendly error UI for Ollama not running / model not found ([#61](https://github.com/quiet-node/thuki/issues/61)) ([6426ea2](https://github.com/quiet-node/thuki/commit/6426ea26e96eb985fa942b68fc8570bdee984159))
* improve context awareness and image handling for better multimodal understanding ([7f64352](https://github.com/quiet-node/thuki/commit/7f643525bceb25154d481c6dd4aa78f4dce89460))
* onboarding flow with permission-gated stage machine ([#65](https://github.com/quiet-node/thuki/issues/65)) ([35497cb](https://github.com/quiet-node/thuki/commit/35497cb8b1ceb7f10533b6323a3c68a8dd361b1b))
* overhaul system prompt and move to dedicated file ([#64](https://github.com/quiet-node/thuki/issues/64)) ([c831c66](https://github.com/quiet-node/thuki/commit/c831c66dcc96a87aed1767eed3093cced4a5db66))
* upgrade to Gemma4 and add runtime model configuration ([#63](https://github.com/quiet-node/thuki/issues/63)) ([5138eac](https://github.com/quiet-node/thuki/commit/5138eac6826fcf94009d8f2a31fe7c37a06cbd9a))


### Bug Fixes

* remove Input Monitoring and suppress native permission popups ([#68](https://github.com/quiet-node/thuki/issues/68)) ([89f06b8](https://github.com/quiet-node/thuki/commit/89f06b87d832dd4acc13de2cba598e7e91135170))
* restore cross-app hotkey via HID tap + active tap options ([#66](https://github.com/quiet-node/thuki/issues/66)) ([8c7f2cd](https://github.com/quiet-node/thuki/commit/8c7f2cd34a42665b6c2b21b8a2beafe2e7f6b76d))

## [0.4.0](https://github.com/quiet-node/thuki/compare/v0.3.0...v0.4.0) (2026-04-07)


### Features

* onboarding screen for macOS permission setup ([#54](https://github.com/quiet-node/thuki/issues/54)) ([d42ae2a](https://github.com/quiet-node/thuki/commit/d42ae2ad00752bafcd95ac7872673ca754fd3e50))


### Bug Fixes

* revert Cargo.lock sync commit to plain git push ([#52](https://github.com/quiet-node/thuki/issues/52)) ([904cdf4](https://github.com/quiet-node/thuki/commit/904cdf44343767d342240712ddc9a43263580af5))

## [0.3.0](https://github.com/quiet-node/thuki/compare/v0.2.1...v0.3.0) (2026-04-06)


### Features

* show AskBar automatically on app launch ([#48](https://github.com/quiet-node/thuki/issues/48)) ([66c994c](https://github.com/quiet-node/thuki/commit/66c994ca75cb71afa6a87e7a3ca9d04eb78e2c9b))


### Bug Fixes

* add Signed-off-by to release-please and Cargo.lock sync commits ([#45](https://github.com/quiet-node/thuki/issues/45)) ([2943f20](https://github.com/quiet-node/thuki/commit/2943f2000f5198a063a164cdd89eeeb5814eb912))
* move signoff to top-level in release-please config ([#47](https://github.com/quiet-node/thuki/issues/47)) ([5a7d076](https://github.com/quiet-node/thuki/commit/5a7d076a196620af6839dd2e9cca9de8e2329d24))
* sync Cargo.lock on release PRs via release workflow ([#43](https://github.com/quiet-node/thuki/issues/43)) ([18f49a4](https://github.com/quiet-node/thuki/commit/18f49a40a3fb944a15beddbc9d1b8c73837add23))
* use GitHub API for Cargo.lock commit to get Verified badge ([#50](https://github.com/quiet-node/thuki/issues/50)) ([cf09593](https://github.com/quiet-node/thuki/commit/cf0959330ebb74b433f35d7ba439b087dd67aeb8))

## [0.2.1](https://github.com/quiet-node/thuki/compare/v0.2.0...v0.2.1) (2026-04-05)


### Bug Fixes

* resolve production screenshot bugs (CSP blob URLs, black screen) ([#41](https://github.com/quiet-node/thuki/issues/41)) ([39da9e8](https://github.com/quiet-node/thuki/commit/39da9e8f87db2ab575c480e71531b0555fa6a8b6))
* sync Cargo.lock to reflect 0.2.0 version bump ([ca17e83](https://github.com/quiet-node/thuki/commit/ca17e83a6bef8de61d5d5dd5cb6a6fc8a049f1ba))

## [0.2.0](https://github.com/quiet-node/thuki/compare/v0.1.0...v0.2.0) (2026-04-05)


### Features

* add /screen slash command with tab-completion and screen capture ([#35](https://github.com/quiet-node/thuki/issues/35)) ([354403a](https://github.com/quiet-node/thuki/commit/354403a9c20eb33e2829de7aece5285cc72fb75a))


### Bug Fixes

* macOS distribution improvements (signing, DMG installer, permissions) ([#36](https://github.com/quiet-node/thuki/issues/36)) ([72b503c](https://github.com/quiet-node/thuki/commit/72b503c7cae2bc50c131d6a8ac12a91c7b56e6d6))

## [0.1.0] - 2026-04-05

### Added

- Floating overlay activated by double-tapping the Control key from any app
- Streaming chat powered by locally running Ollama models
- Multi-turn conversation with full context retention
- Conversation history with SQLite persistence; revisit and continue past sessions
- Image and screenshot input: paste or drag images directly into the chat
- Docker sandbox with capability dropping, read-only model volume, and localhost-only networking
- macOS NSPanel integration for fullscreen-app overlay
- Tray icon with show/hide and quit controls
- Automatic window resizing driven by content height
- Markdown rendering via Streamdown with XSS protection
- Cancel in-flight generation with a stop button
- History panel with search, save/unsave, and conversation switching

[Unreleased]: https://github.com/quiet-node/thuki/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/quiet-node/thuki/releases/tag/v0.1.0
