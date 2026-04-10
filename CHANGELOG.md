# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0](https://github.com/quiet-node/thuki/compare/v0.5.1...v0.6.0) (2026-04-10)


### Features

* **activator:** switch activation hotkey from double-Option to double-Command ([1414f48](https://github.com/quiet-node/thuki/commit/1414f482af3f310bf6ac9e53724b0f8730a74d7c))
* add /screen slash command with tab-completion and screen capture ([#35](https://github.com/quiet-node/thuki/issues/35)) ([072de70](https://github.com/quiet-node/thuki/commit/072de70a5812e5a4af1403c23ab94b58e7fa36c5))
* add chat window for conversations ([1234238](https://github.com/quiet-node/thuki/commit/1234238e8b7bf62d982d4d3bc4859c4f9b09ad5f))
* add test:all:coverage script for combined frontend and backend coverage enforcement ([d26a16d](https://github.com/quiet-node/thuki/commit/d26a16dcc9dc43d5a010990349e15fc21b4b22a1))
* allow for Thuki overlap to spawn on fullscreen apps ([eb847f5](https://github.com/quiet-node/thuki/commit/eb847f57a25fdd10cd36572cf8e0e626b9ff10bc))
* centralize dragging, preserve focus, and tighten shadows ([edc4a0a](https://github.com/quiet-node/thuki/commit/edc4a0aafc1539d4baccc812d81333e20b691370))
* commits should produce 0.x.0 releases (e.g. 0.2.0), not patch ([248d147](https://github.com/quiet-node/thuki/commit/248d147ede213ebb53e0d26fe0c078d5f98dae2d))
* context-aware AskBar with smart positioning ([5990401](https://github.com/quiet-node/thuki/commit/59904017a8b677c1fb56cf1b4bacdd5fe9b4afe3))
* conversation history frontend ([#19](https://github.com/quiet-node/thuki/issues/19)) ([63e24aa](https://github.com/quiet-node/thuki/commit/63e24aa726b895a7455dc92f456f5e6449475cea))
* friendly error UI for Ollama not running / model not found ([#61](https://github.com/quiet-node/thuki/issues/61)) ([c86b291](https://github.com/quiet-node/thuki/commit/c86b291d62134130711491922a46ee882bfe5d18))
* history panel UX improvements ([#26](https://github.com/quiet-node/thuki/issues/26)) ([d1dffd3](https://github.com/quiet-node/thuki/commit/d1dffd3ff74a1eb28f6390c468629042ea678a9b))
* holistically updated chat interface overlay ([fa7b6c5](https://github.com/quiet-node/thuki/commit/fa7b6c5674a01cfb9478e8fc9316b9c2d1a90421))
* image and screenshot input support ([#28](https://github.com/quiet-node/thuki/issues/28)) ([f27f84e](https://github.com/quiet-node/thuki/commit/f27f84ee4ea781284f179b4a91a261ef4bdf9df2))
* implement professional overlay activator and nspanel integration ([ffa7d7b](https://github.com/quiet-node/thuki/commit/ffa7d7b77ba0d8a7094d0b2ad5ea407e23d38e2a))
* implement secure, isolated Docker sandbox for LLM inference ([7056d56](https://github.com/quiet-node/thuki/commit/7056d564ab8bfcbc3c4105339b9cee1600a3b121))
* improve context awareness and image handling for better multimodal understanding ([d719fbf](https://github.com/quiet-node/thuki/commit/d719fbfbd9fea6526d3cfad25b24effb791440c5))
* initial commit ([9144d48](https://github.com/quiet-node/thuki/commit/9144d48e43bcbde808f136faedcea950687c7714))
* multi-turn conversation via Ollama /api/chat ([#16](https://github.com/quiet-node/thuki/issues/16)) ([3fc19c8](https://github.com/quiet-node/thuki/commit/3fc19c808f9cd15d420963e8d48d9a61f1c4421a))
* onboarding flow with permission-gated stage machine ([#65](https://github.com/quiet-node/thuki/issues/65)) ([675c423](https://github.com/quiet-node/thuki/commit/675c423c1a0bc818ec7a106de743ebe21a1868c0))
* onboarding screen for macOS permission setup ([#54](https://github.com/quiet-node/thuki/issues/54)) ([f26f6c8](https://github.com/quiet-node/thuki/commit/f26f6c89ecca55986410597a75ce2ac6c9a57926))
* open source readiness ([#32](https://github.com/quiet-node/thuki/issues/32)) ([edc22b1](https://github.com/quiet-node/thuki/commit/edc22b13be72ea267bcc8c2c931bf40d34b5bf80))
* overhaul system prompt and move to dedicated file ([#64](https://github.com/quiet-node/thuki/issues/64)) ([94ebed4](https://github.com/quiet-node/thuki/commit/94ebed44bb91e1127c7d6cc0a420a9ff1831047a))
* replace application icons with new mascot logo and update tray resolution ([bfcc38a](https://github.com/quiet-node/thuki/commit/bfcc38af13d3cc8eaa3f186d29e88c325aa31f27))
* replace react-markdown with streamdown for jitter-free streaming ([#17](https://github.com/quiet-node/thuki/issues/17)) ([5a682d9](https://github.com/quiet-node/thuki/commit/5a682d996315eb2cecb44c080c2255c2f8e8a474))
* screenshot capture for image input ([#31](https://github.com/quiet-node/thuki/issues/31)) ([88f0fef](https://github.com/quiet-node/thuki/commit/88f0fef2578f7e917ab6c48c41fe376c6df871cf))
* secure ollama integration with lean architecture ([dfbc43b](https://github.com/quiet-node/thuki/commit/dfbc43b36dc3782afa0bc872b00ffdb9a551dd81))
* selected-text quote display with .env-driven configuration ([#1](https://github.com/quiet-node/thuki/issues/1)) ([c4e6118](https://github.com/quiet-node/thuki/commit/c4e6118c3b861f6c579069f6aaf1213524811c94))
* show AskBar automatically on app launch ([#48](https://github.com/quiet-node/thuki/issues/48)) ([fe770bf](https://github.com/quiet-node/thuki/commit/fe770bf5e1ad8a3a29c9badcd54107221f6d8b0b))
* spring-driven morph animation for askbar-to-chat transition ([#11](https://github.com/quiet-node/thuki/issues/11)) ([5bcec5f](https://github.com/quiet-node/thuki/commit/5bcec5f9bd2ad33897bd0c41a911a1626c90840f))
* SQLite persistence layer for conversation history (backend) ([#18](https://github.com/quiet-node/thuki/issues/18)) ([1d5b49b](https://github.com/quiet-node/thuki/commit/1d5b49b94c28e8846599a686287d4c2e1f9b7286))
* stream cancellation with stop generating button ([#13](https://github.com/quiet-node/thuki/issues/13)) ([9426c03](https://github.com/quiet-node/thuki/commit/9426c0349e063a82a79c5042edb33b15dd384296))
* UI polish — chat redesign, spiral loader, smooth upward animation ([#21](https://github.com/quiet-node/thuki/issues/21)) ([4576aed](https://github.com/quiet-node/thuki/commit/4576aed8f481a2f8b8781683d7853136a4f6a1fe))
* UI polish, conversation save/unsave, and window positioning ([#24](https://github.com/quiet-node/thuki/issues/24)) ([dfc37e0](https://github.com/quiet-node/thuki/commit/dfc37e0f10a8d4991f15863345217dae612db984))
* **ui:** added copy button for chat bubble ([ef829bb](https://github.com/quiet-node/thuki/commit/ef829bb72469dfd4775c70abd542a27c2dacec37))
* update activator key to Ctrl and adjust default ask bar position ([#23](https://github.com/quiet-node/thuki/issues/23)) ([c9f4753](https://github.com/quiet-node/thuki/commit/c9f4753e1c72e592333e1b7071884ea0bb049416))
* upgrade to Gemma4 and add runtime model configuration ([#63](https://github.com/quiet-node/thuki/issues/63)) ([448a1a8](https://github.com/quiet-node/thuki/commit/448a1a8751ed62762320d41b13ab9fec03ab8fe5))


### Bug Fixes

* add Signed-off-by to release-please and Cargo.lock sync commits ([#45](https://github.com/quiet-node/thuki/issues/45)) ([789d556](https://github.com/quiet-node/thuki/commit/789d556c6a1519f2897e6d34e50523d7ecc3f60b))
* auto-scroll stops following streaming after max height reached ([#12](https://github.com/quiet-node/thuki/issues/12)) ([42d1778](https://github.com/quiet-node/thuki/commit/42d17783db8db48a402d2f74463d1a9ecbae5e8d))
* cancel active streaming on overlay hide and app quit ([#73](https://github.com/quiet-node/thuki/issues/73)) ([170f4c0](https://github.com/quiet-node/thuki/commit/170f4c04a619f96c4b9e486cbd9bb748a4fd00b7))
* eliminate streaming jitter during upward window growth ([#10](https://github.com/quiet-node/thuki/issues/10)) ([cdffe40](https://github.com/quiet-node/thuki/commit/cdffe409427842eb5f9a07eb48a94f44e2392972))
* enable drag-and-drop image support in Thuki window ([8f7b95e](https://github.com/quiet-node/thuki/commit/8f7b95e40d26a011190a581001a8507f86a7914e))
* fix auto scroll + regression tests for incremental resize ([#14](https://github.com/quiet-node/thuki/issues/14)) ([b2865b1](https://github.com/quiet-node/thuki/commit/b2865b1f2a3cc537a00099ba9d4ebc4912c3f81b))
* hide search input when switch confirmation is shown ([#27](https://github.com/quiet-node/thuki/issues/27)) ([5e3219e](https://github.com/quiet-node/thuki/commit/5e3219e1ab9c78c4d590d8960e1a60ca919b888a))
* macOS distribution improvements (signing, DMG installer, permissions) ([#36](https://github.com/quiet-node/thuki/issues/36)) ([7bba18f](https://github.com/quiet-node/thuki/commit/7bba18f4691f4717b144e15392609130d84c5309))
* move signoff to top-level in release-please config ([#47](https://github.com/quiet-node/thuki/issues/47)) ([91d2c12](https://github.com/quiet-node/thuki/commit/91d2c1242e1c0964052f6bcf876f3f9078fbb43e))
* preserve scroll position when streaming finishes ([#70](https://github.com/quiet-node/thuki/issues/70)) ([682e513](https://github.com/quiet-node/thuki/commit/682e513d88562f1d67636bddcb19452976ff98d5))
* preserve whitespace formatting in user chat bubbles ([#30](https://github.com/quiet-node/thuki/issues/30)) ([4e37a75](https://github.com/quiet-node/thuki/commit/4e37a752a4f5e89186a3cfc68a19bd047ae3a47e))
* raise Vite chunkSizeWarningLimit to suppress bundle size warning ([#15](https://github.com/quiet-node/thuki/issues/15)) ([81598a3](https://github.com/quiet-node/thuki/commit/81598a384caf91874164049f04a5a64ab69f9751))
* remove Input Monitoring and suppress native permission popups ([#68](https://github.com/quiet-node/thuki/issues/68)) ([ba1df94](https://github.com/quiet-node/thuki/commit/ba1df9454e4e897cf249e99f42ec66eaa8c28cb4))
* replace anchor system with simple screen-bottom growth detection ([#74](https://github.com/quiet-node/thuki/issues/74)) ([c03f974](https://github.com/quiet-node/thuki/commit/c03f97491c5a8db0ee3953b5f3b696e75904b8e4))
* replace marked+DOMPurify with react-markdown and add stable message keys ([4ce15d2](https://github.com/quiet-node/thuki/commit/4ce15d2d9cabf530d62d79d07fd0580a0252043a))
* replace marked+DOMPurify with react-markdown, add stable message keys ([#9](https://github.com/quiet-node/thuki/issues/9)) ([4ce15d2](https://github.com/quiet-node/thuki/commit/4ce15d2d9cabf530d62d79d07fd0580a0252043a))
* resolve production screenshot bugs (CSP blob URLs, black screen) ([#41](https://github.com/quiet-node/thuki/issues/41)) ([9debfce](https://github.com/quiet-node/thuki/commit/9debfce39c5c55c375e96356767c07a689c1c5a2))
* restore cross-app hotkey via HID tap + active tap options ([#66](https://github.com/quiet-node/thuki/issues/66)) ([7636e3a](https://github.com/quiet-node/thuki/commit/7636e3adeba5b8197bf5af69e4f487c6467bc266))
* retain conversation context when generation is cancelled ([#25](https://github.com/quiet-node/thuki/issues/25)) ([390fcf1](https://github.com/quiet-node/thuki/commit/390fcf1d0e765d1199d26e057b71aa3fd306edcf))
* revert Cargo.lock commit to plain git push ([27cad8d](https://github.com/quiet-node/thuki/commit/27cad8d930a9735a868a6fa17360d68442957899))
* revert Cargo.lock sync commit to plain git push ([#52](https://github.com/quiet-node/thuki/issues/52)) ([27cad8d](https://github.com/quiet-node/thuki/commit/27cad8d930a9735a868a6fa17360d68442957899))
* sync Cargo.lock and add workflow to keep it in sync on release PRs ([5eb093d](https://github.com/quiet-node/thuki/commit/5eb093d35240c5d43b0144ccae453d5cdba36d59))
* sync Cargo.lock on release PRs via release workflow ([#43](https://github.com/quiet-node/thuki/issues/43)) ([5eb093d](https://github.com/quiet-node/thuki/commit/5eb093d35240c5d43b0144ccae453d5cdba36d59))
* sync Cargo.lock to reflect 0.2.0 version bump ([4c2572a](https://github.com/quiet-node/thuki/commit/4c2572aff5710efceb92062bcaa32e405ce11b87))
* **ui:** enable text selection in chat bubbles while maintaining window drag ([5b8f503](https://github.com/quiet-node/thuki/commit/5b8f5033b485e9ae34be78756cbd08430b5def19))
* use GitHub API for Cargo.lock commit to get Verified badge ([#50](https://github.com/quiet-node/thuki/issues/50)) ([2cc31cd](https://github.com/quiet-node/thuki/commit/2cc31cd816bceb51f78e4be7e31cdaf327d22085))
* use wheel events for auto-scroll to prevent layout-induced false negatives ([42d1778](https://github.com/quiet-node/thuki/commit/42d17783db8db48a402d2f74463d1a9ecbae5e8d))

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
