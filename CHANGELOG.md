# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## 1.0.0 (2026-04-05)


### Features

* **activator:** switch activation hotkey from double-Option to double-Command ([fbcb268](https://github.com/quiet-node/thuki/commit/fbcb26837c59856545f4583d079668f61a60069d))
* add chat window for conversations ([b3586d1](https://github.com/quiet-node/thuki/commit/b3586d1abb2b894c0bcd3392bdb36dd37fa2f285))
* add test:all:coverage script for combined frontend and backend coverage enforcement ([38b1492](https://github.com/quiet-node/thuki/commit/38b14923a0bb52cf7ad697cc93595eb3f94069f5))
* allow for Thuki overlap to spawn on fullscreen apps ([43cce25](https://github.com/quiet-node/thuki/commit/43cce2536d9520c8be9c420afd01a0734154ae13))
* centralize dragging, preserve focus, and tighten shadows ([3d51acc](https://github.com/quiet-node/thuki/commit/3d51acc0dff676c94459d093575b45c363e12062))
* context-aware AskBar with smart positioning ([8f4cf20](https://github.com/quiet-node/thuki/commit/8f4cf20e1a0f06638b8f3bcce493b8f6d61b9533))
* conversation history frontend ([#19](https://github.com/quiet-node/thuki/issues/19)) ([e4fe3bc](https://github.com/quiet-node/thuki/commit/e4fe3bca92e15145eb8a8a5cbca3ab95d58e6d99))
* history panel UX improvements ([#26](https://github.com/quiet-node/thuki/issues/26)) ([2c3de16](https://github.com/quiet-node/thuki/commit/2c3de161c72984d9a26d4a4e724555d9fd9ce8b3))
* holistically updated chat interface overlay ([fd798ef](https://github.com/quiet-node/thuki/commit/fd798ef476f809ecfb7ef7aa6ed89e03262997c1))
* image and screenshot input support ([#28](https://github.com/quiet-node/thuki/issues/28)) ([545da0e](https://github.com/quiet-node/thuki/commit/545da0ef8adeb63277ee333cd8318adf7c11795b))
* implement professional overlay activator and nspanel integration ([b014ef7](https://github.com/quiet-node/thuki/commit/b014ef7cacbfdc4b1c9a82de81426a51aeb97af7))
* implement secure, isolated Docker sandbox for LLM inference ([c1d55c0](https://github.com/quiet-node/thuki/commit/c1d55c0b2df809bbaf47c18d22649e94b5c0ef8a))
* initial commit ([b785350](https://github.com/quiet-node/thuki/commit/b785350baf24422cb9d7ed44be55720e18e260ff))
* initial release of Thuki ([a950442](https://github.com/quiet-node/thuki/commit/a95044218c76ab021b58367a70c6c9c8f1720134))
* multi-turn conversation via Ollama /api/chat ([#16](https://github.com/quiet-node/thuki/issues/16)) ([f79b403](https://github.com/quiet-node/thuki/commit/f79b40301a67634b11e1258b60de9d47b5378467))
* open source readiness ([#32](https://github.com/quiet-node/thuki/issues/32)) ([bc21281](https://github.com/quiet-node/thuki/commit/bc2128145a981ac15f6e1a90a27616c64c669c97))
* replace application icons with new mascot logo and update tray resolution ([8142209](https://github.com/quiet-node/thuki/commit/81422097f786405d33bf4928c330f0a06877c541))
* replace react-markdown with streamdown for jitter-free streaming ([#17](https://github.com/quiet-node/thuki/issues/17)) ([1f41737](https://github.com/quiet-node/thuki/commit/1f41737f162ecb35e54ef3e6413251cb990b6047))
* screenshot capture for image input ([#31](https://github.com/quiet-node/thuki/issues/31)) ([dba91cd](https://github.com/quiet-node/thuki/commit/dba91cdf7a7386d71c93585175203a51da120dcc))
* secure ollama integration with lean architecture ([ab5dcce](https://github.com/quiet-node/thuki/commit/ab5dcce5d7d013e0814a1a5aa69773233ab81493))
* selected-text quote display with .env-driven configuration ([#1](https://github.com/quiet-node/thuki/issues/1)) ([01e76b8](https://github.com/quiet-node/thuki/commit/01e76b89a87c43f5605d1fbd365a9de4e4d7fb1d))
* spring-driven morph animation for askbar-to-chat transition ([#11](https://github.com/quiet-node/thuki/issues/11)) ([512b7f2](https://github.com/quiet-node/thuki/commit/512b7f28500e6ac075274759f293cdc6100063a5))
* SQLite persistence layer for conversation history (backend) ([#18](https://github.com/quiet-node/thuki/issues/18)) ([0ebe031](https://github.com/quiet-node/thuki/commit/0ebe0311fb49dc150bc332abb7c337974d667983))
* stream cancellation with stop generating button ([#13](https://github.com/quiet-node/thuki/issues/13)) ([0eb4318](https://github.com/quiet-node/thuki/commit/0eb43182799f8035038c61010a27b47c39876dfa))
* UI polish — chat redesign, spiral loader, smooth upward animation ([#21](https://github.com/quiet-node/thuki/issues/21)) ([4dfaba9](https://github.com/quiet-node/thuki/commit/4dfaba95b59cf7fb11a3b329af60b8da872fedb8))
* UI polish, conversation save/unsave, and window positioning ([#24](https://github.com/quiet-node/thuki/issues/24)) ([0789f83](https://github.com/quiet-node/thuki/commit/0789f83cfe733eec0bb2b173bad9ce12e891c447))
* **ui:** added copy button for chat bubble ([d72d7e4](https://github.com/quiet-node/thuki/commit/d72d7e45514f702176a33625192b72a3a3542801))
* update activator key to Ctrl and adjust default ask bar position ([#23](https://github.com/quiet-node/thuki/issues/23)) ([526bde4](https://github.com/quiet-node/thuki/commit/526bde42a761482c21c40d77bcd244d7492704af))


### Bug Fixes

* auto-scroll stops following streaming after max height reached ([#12](https://github.com/quiet-node/thuki/issues/12)) ([d84dc99](https://github.com/quiet-node/thuki/commit/d84dc99f1eef9729e67d3eb2626a8a9292ccbc3e))
* eliminate streaming jitter during upward window growth ([#10](https://github.com/quiet-node/thuki/issues/10)) ([0213b34](https://github.com/quiet-node/thuki/commit/0213b341a72d5594fdc24ebad0d870168aac62e3))
* enable drag-and-drop image support in Thuki window ([bb3eaee](https://github.com/quiet-node/thuki/commit/bb3eaee4b75697d12a7600a8f1d2ddf5bc5b823c))
* fix auto scroll + regression tests for incremental resize ([#14](https://github.com/quiet-node/thuki/issues/14)) ([55c2dc3](https://github.com/quiet-node/thuki/commit/55c2dc3163c60b0f137f3994d21d44e783cf13ca))
* hide search input when switch confirmation is shown ([#27](https://github.com/quiet-node/thuki/issues/27)) ([716d504](https://github.com/quiet-node/thuki/commit/716d5044c40e95f557a8fdfc614a74324c34c07d))
* preserve whitespace formatting in user chat bubbles ([#30](https://github.com/quiet-node/thuki/issues/30)) ([14d44e0](https://github.com/quiet-node/thuki/commit/14d44e06b5cfc2b4457cd23472562d0f490bc3ad))
* raise Vite chunkSizeWarningLimit to suppress bundle size warning ([#15](https://github.com/quiet-node/thuki/issues/15)) ([61351d1](https://github.com/quiet-node/thuki/commit/61351d1898ed272da5c0fff3c01cdd7d51d8086f))
* replace marked+DOMPurify with react-markdown, add stable message keys ([#9](https://github.com/quiet-node/thuki/issues/9)) ([2d620ec](https://github.com/quiet-node/thuki/commit/2d620ec95ddc725edabf7567b3e875ccab63cc1f))
* retain conversation context when generation is cancelled ([#25](https://github.com/quiet-node/thuki/issues/25)) ([3e6c846](https://github.com/quiet-node/thuki/commit/3e6c846f93d643ad981322bf6d3ea7d29656e88b))
* **ui:** enable text selection in chat bubbles while maintaining window drag ([4d060c3](https://github.com/quiet-node/thuki/commit/4d060c3324097301bf3f42634a2e63db68acfa2d))

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
