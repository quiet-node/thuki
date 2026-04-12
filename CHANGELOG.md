# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.0](https://github.com/quiet-node/thuki/compare/v0.5.1...v0.6.0) (2026-04-12)


### Features

* **activator:** switch activation hotkey from double-Option to double-Command ([bfa179a](https://github.com/quiet-node/thuki/commit/bfa179a57f8126940683d72c3f8e418b618cbc48))
* add /screen slash command with tab-completion and screen capture ([#35](https://github.com/quiet-node/thuki/issues/35)) ([d6ff460](https://github.com/quiet-node/thuki/commit/d6ff46055f88881187e7961317ddc7899795c3d1))
* add chat window for conversations ([4007d58](https://github.com/quiet-node/thuki/commit/4007d58331acc8e67f9e8bdeadce0def9d7c677c))
* add test:all:coverage script for combined frontend and backend coverage enforcement ([88c7850](https://github.com/quiet-node/thuki/commit/88c78506bf0732d41a1007945fa6383d7fc2ba4e))
* allow for Thuki overlap to spawn on fullscreen apps ([979f496](https://github.com/quiet-node/thuki/commit/979f496d6a68d46b6c9098928e3e518178930706))
* centralize dragging, preserve focus, and tighten shadows ([f00d113](https://github.com/quiet-node/thuki/commit/f00d1134a462dc53f2c111b11f51ba96eb997810))
* commits should produce 0.x.0 releases (e.g. 0.2.0), not patch ([a6d9fee](https://github.com/quiet-node/thuki/commit/a6d9fee52f5f4880d9a09331da0aa453174fee0e))
* context-aware AskBar with smart positioning ([4b9543d](https://github.com/quiet-node/thuki/commit/4b9543d8bd0b9818f7fca4dcf23be412244c5e1c))
* conversation history frontend ([#19](https://github.com/quiet-node/thuki/issues/19)) ([7903d21](https://github.com/quiet-node/thuki/commit/7903d2116a8e1f209c557fc8942e9285f0541507))
* friendly error UI for Ollama not running / model not found ([#61](https://github.com/quiet-node/thuki/issues/61)) ([83e397d](https://github.com/quiet-node/thuki/commit/83e397d0a8450aef75477549393bf532427a3d05))
* history panel UX improvements ([#26](https://github.com/quiet-node/thuki/issues/26)) ([57ef3d2](https://github.com/quiet-node/thuki/commit/57ef3d22c7ff0358714f0b5cdf9e8d313273e1fa))
* holistically updated chat interface overlay ([317cc8e](https://github.com/quiet-node/thuki/commit/317cc8e34c59b316edcc17e7da3912b1b4b7d4c5))
* image and screenshot input support ([#28](https://github.com/quiet-node/thuki/issues/28)) ([e0d3917](https://github.com/quiet-node/thuki/commit/e0d3917fb024c5f92ccc65e8da57e62831b0cd11))
* implement professional overlay activator and nspanel integration ([0c165b2](https://github.com/quiet-node/thuki/commit/0c165b20a11d963e3b91f6ab13380e6e7db2f017))
* implement secure, isolated Docker sandbox for LLM inference ([32e9ad3](https://github.com/quiet-node/thuki/commit/32e9ad33a7d0d4a406df1a85197609baf2a5cbbc))
* improve context awareness and image handling for better multimodal understanding ([7f19f23](https://github.com/quiet-node/thuki/commit/7f19f2331110c22cbd7ee9e87416c2ed9e19ac20))
* initial commit ([090c764](https://github.com/quiet-node/thuki/commit/090c764d75cb4f74a623bb7815b0841a17bdd16a))
* multi-turn conversation via Ollama /api/chat ([#16](https://github.com/quiet-node/thuki/issues/16)) ([01930d7](https://github.com/quiet-node/thuki/commit/01930d774c1fba42061a92d442e889105179bb8f))
* onboarding flow with permission-gated stage machine ([#65](https://github.com/quiet-node/thuki/issues/65)) ([d7845a3](https://github.com/quiet-node/thuki/commit/d7845a38e81301079a0a14786e200eddc62164e5))
* onboarding screen for macOS permission setup ([#54](https://github.com/quiet-node/thuki/issues/54)) ([e6b9594](https://github.com/quiet-node/thuki/commit/e6b9594a54b640aeac49e6503174a4b69a93904a))
* open source readiness ([#32](https://github.com/quiet-node/thuki/issues/32)) ([082a8d5](https://github.com/quiet-node/thuki/commit/082a8d5e06753851dc8adacf25480ab3e9a2f49c))
* overhaul system prompt and move to dedicated file ([#64](https://github.com/quiet-node/thuki/issues/64)) ([698ae20](https://github.com/quiet-node/thuki/commit/698ae20f704a5390d9ce87359419c4934613e52b))
* replace application icons with new mascot logo and update tray resolution ([c0e6b8e](https://github.com/quiet-node/thuki/commit/c0e6b8efd09a086132b18a2cf26562d53011cf3a))
* replace react-markdown with streamdown for jitter-free streaming ([#17](https://github.com/quiet-node/thuki/issues/17)) ([dc9d306](https://github.com/quiet-node/thuki/commit/dc9d3064dc0b118bdd2d93d9d200cd64e2d04c78))
* screenshot capture for image input ([#31](https://github.com/quiet-node/thuki/issues/31)) ([78f28e6](https://github.com/quiet-node/thuki/commit/78f28e6aef63e338e675bda763ab7c589bca8419))
* secure ollama integration with lean architecture ([8724e7d](https://github.com/quiet-node/thuki/commit/8724e7d28f5869f430ac1780185c6282b672182d))
* selected-text quote display with .env-driven configuration ([#1](https://github.com/quiet-node/thuki/issues/1)) ([e66b4d1](https://github.com/quiet-node/thuki/commit/e66b4d1704df3cb38641117e381dac378afd3d57))
* show AskBar automatically on app launch ([#48](https://github.com/quiet-node/thuki/issues/48)) ([f470f2a](https://github.com/quiet-node/thuki/commit/f470f2a57531578500e72c7985e8d26a03b3341f))
* spring-driven morph animation for askbar-to-chat transition ([#11](https://github.com/quiet-node/thuki/issues/11)) ([b7edfa1](https://github.com/quiet-node/thuki/commit/b7edfa1e88e624b44f1453956f51d9e3bbf0fa23))
* SQLite persistence layer for conversation history (backend) ([#18](https://github.com/quiet-node/thuki/issues/18)) ([b7cd545](https://github.com/quiet-node/thuki/commit/b7cd545f520825b85192e108e75585683b53f686))
* stream cancellation with stop generating button ([#13](https://github.com/quiet-node/thuki/issues/13)) ([0b06ab3](https://github.com/quiet-node/thuki/commit/0b06ab355f75dbe6830ab9c89da52ed8d14b99f5))
* UI polish — chat redesign, spiral loader, smooth upward animation ([#21](https://github.com/quiet-node/thuki/issues/21)) ([f1c1b03](https://github.com/quiet-node/thuki/commit/f1c1b03c0008755dd9df3ee608368c88950a5249))
* UI polish, conversation save/unsave, and window positioning ([#24](https://github.com/quiet-node/thuki/issues/24)) ([1dabcd3](https://github.com/quiet-node/thuki/commit/1dabcd34dfbe5e48854c990ddadc975110d516f1))
* **ui:** added copy button for chat bubble ([fa2b40c](https://github.com/quiet-node/thuki/commit/fa2b40cc62146110545400d89e8d5b0c2e774a51))
* update activator key to Ctrl and adjust default ask bar position ([#23](https://github.com/quiet-node/thuki/issues/23)) ([e866f4c](https://github.com/quiet-node/thuki/commit/e866f4c1af75095aaf2eb1dbc6744aba1308d8be))
* upgrade to Gemma4 and add runtime model configuration ([#63](https://github.com/quiet-node/thuki/issues/63)) ([b164ac0](https://github.com/quiet-node/thuki/commit/b164ac0a88b48ac66b10299ab8e2004a6b557609))


### Bug Fixes

* add Signed-off-by to release-please and Cargo.lock sync commits ([#45](https://github.com/quiet-node/thuki/issues/45)) ([c33b08e](https://github.com/quiet-node/thuki/commit/c33b08e33fd06fab19a591cfc14fe7c4a662c0a7))
* auto-scroll stops following streaming after max height reached ([#12](https://github.com/quiet-node/thuki/issues/12)) ([7855f5d](https://github.com/quiet-node/thuki/commit/7855f5d359529cee6ede3dd4ec9951680991114c))
* cancel active streaming on overlay hide and app quit ([#73](https://github.com/quiet-node/thuki/issues/73)) ([b678879](https://github.com/quiet-node/thuki/commit/b678879aa029ea1395204ae867b4c077397833cb))
* eliminate streaming jitter during upward window growth ([#10](https://github.com/quiet-node/thuki/issues/10)) ([f1a0332](https://github.com/quiet-node/thuki/commit/f1a0332481dedd2bac4d177516f52b75f32c012b))
* enable drag-and-drop image support in Thuki window ([dab5c28](https://github.com/quiet-node/thuki/commit/dab5c28adbaf31712a7945e79bcd4ad2c609065c))
* enlarge close button hit area to fix unreliable click ([#82](https://github.com/quiet-node/thuki/issues/82)) ([a829858](https://github.com/quiet-node/thuki/commit/a829858b8458e70fa704c0174e0589cdb4728feb))
* fix auto scroll + regression tests for incremental resize ([#14](https://github.com/quiet-node/thuki/issues/14)) ([534639a](https://github.com/quiet-node/thuki/commit/534639ac14d3031329f77a85a6d81b61ea265233))
* hide search input when switch confirmation is shown ([#27](https://github.com/quiet-node/thuki/issues/27)) ([29b6877](https://github.com/quiet-node/thuki/commit/29b68776ffeac806e579a3c0d3a5ce5844c59bb7))
* macOS distribution improvements (signing, DMG installer, permissions) ([#36](https://github.com/quiet-node/thuki/issues/36)) ([8a858d1](https://github.com/quiet-node/thuki/commit/8a858d10ff657d60f5c784e2ec50bcaffc803491))
* move signoff to top-level in release-please config ([#47](https://github.com/quiet-node/thuki/issues/47)) ([4bbd5db](https://github.com/quiet-node/thuki/commit/4bbd5db96c7c5944446c0401605038b1ad446a17))
* preserve scroll position when streaming finishes ([#70](https://github.com/quiet-node/thuki/issues/70)) ([6632995](https://github.com/quiet-node/thuki/commit/66329953a0e8eb0a691d9c2cefb7ed4560ddbf1c))
* preserve whitespace formatting in user chat bubbles ([#30](https://github.com/quiet-node/thuki/issues/30)) ([9254dde](https://github.com/quiet-node/thuki/commit/9254ddea5aaa010355b8775b9d1cf89c0f44d124))
* raise Vite chunkSizeWarningLimit to suppress bundle size warning ([#15](https://github.com/quiet-node/thuki/issues/15)) ([8abf186](https://github.com/quiet-node/thuki/commit/8abf18617c653b144dbbf3a5ae27390aff20176a))
* remove Input Monitoring and suppress native permission popups ([#68](https://github.com/quiet-node/thuki/issues/68)) ([b6e7fa3](https://github.com/quiet-node/thuki/commit/b6e7fa3d34991ac02e7f61b49b41662fe2015ad6))
* replace anchor system with simple screen-bottom growth detection ([#74](https://github.com/quiet-node/thuki/issues/74)) ([85883d5](https://github.com/quiet-node/thuki/commit/85883d5cba808d739506ce33db08fe21b294597d))
* replace marked+DOMPurify with react-markdown and add stable message keys ([cff620e](https://github.com/quiet-node/thuki/commit/cff620e4fdf622c90e664b0028a558b90918005a))
* replace marked+DOMPurify with react-markdown, add stable message keys ([#9](https://github.com/quiet-node/thuki/issues/9)) ([cff620e](https://github.com/quiet-node/thuki/commit/cff620e4fdf622c90e664b0028a558b90918005a))
* resolve production screenshot bugs (CSP blob URLs, black screen) ([#41](https://github.com/quiet-node/thuki/issues/41)) ([d67a154](https://github.com/quiet-node/thuki/commit/d67a154016730311748546213235a92f04505864))
* restore cross-app hotkey via HID tap + active tap options ([#66](https://github.com/quiet-node/thuki/issues/66)) ([802eac2](https://github.com/quiet-node/thuki/commit/802eac2d20dd7db1243fbabfd4192c315972f91b))
* retain conversation context when generation is cancelled ([#25](https://github.com/quiet-node/thuki/issues/25)) ([c825dff](https://github.com/quiet-node/thuki/commit/c825dff509875b68001b3425783c779a83d27277))
* revert Cargo.lock commit to plain git push ([8ec9b8d](https://github.com/quiet-node/thuki/commit/8ec9b8d807883442753dd5e0a090432c0505248f))
* revert Cargo.lock sync commit to plain git push ([#52](https://github.com/quiet-node/thuki/issues/52)) ([8ec9b8d](https://github.com/quiet-node/thuki/commit/8ec9b8d807883442753dd5e0a090432c0505248f))
* sync Cargo.lock and add workflow to keep it in sync on release PRs ([b02a299](https://github.com/quiet-node/thuki/commit/b02a299bcca9aba14eaabb9994404edfbd2cafab))
* sync Cargo.lock on release PRs via release workflow ([#43](https://github.com/quiet-node/thuki/issues/43)) ([b02a299](https://github.com/quiet-node/thuki/commit/b02a299bcca9aba14eaabb9994404edfbd2cafab))
* sync Cargo.lock to reflect 0.2.0 version bump ([fc11cbb](https://github.com/quiet-node/thuki/commit/fc11cbbb59a6ee2c139af5d610dd4cdd95eeee2c))
* **ui:** enable text selection in chat bubbles while maintaining window drag ([3095156](https://github.com/quiet-node/thuki/commit/30951569f8149f34edb9031e6ffbe30fefc0e5b6))
* use GitHub API for Cargo.lock commit to get Verified badge ([#50](https://github.com/quiet-node/thuki/issues/50)) ([afccd57](https://github.com/quiet-node/thuki/commit/afccd57b66303206238cffbf367cfebddeb1cbf5))
* use wheel events for auto-scroll to prevent layout-induced false negatives ([7855f5d](https://github.com/quiet-node/thuki/commit/7855f5d359529cee6ede3dd4ec9951680991114c))

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
