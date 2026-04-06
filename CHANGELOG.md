# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
