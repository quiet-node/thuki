# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- **Unified trace recorder.** Records every chat conversation and `/search` session as JSON-Lines under `app_data_dir/traces/{chat,search}/<conversation_id>.jsonl`. Off by default; toggle from Settings or set `[debug] trace_enabled = true` in `config.toml`.

### Changed

- **BREAKING**: Renamed `[debug] search_trace_enabled` to `trace_enabled` (now covers both chat and search). Rename the field in your `config.toml` after upgrading. Trace file layout also changed to `traces/{chat,search}/<conversation_id>.jsonl`.
- **Inference providers.** Thuki now reaches models through a typed provider list instead of a single hardcoded Ollama endpoint. The `[inference]` section gains `active_provider` and a `[[inference.providers]]` array; each provider keeps its own selected model. Fresh installs default to the bundled **Built-in (Thuki)** engine, with **Ollama** available as an optional provider. Existing Ollama users are migrated automatically: a legacy flat `ollama_url` becomes the Ollama provider's `base_url`, and the previously selected model is carried over, so nothing changes for them. Settings gains a Providers section (editable Ollama URL with a non-local-server warning, per-provider model picker).
- The internal inference command/hook/error model were renamed to be engine-agnostic: `ask_ollama` → `ask_model`, the `useOllama` hook → `useModel`, and `OllamaError`/`OllamaErrorKind` → `EngineError`/`EngineErrorKind` (the `NotRunning` variant is now `EngineUnreachable`). External callers that invoked `ask_ollama` directly must update to `ask_model`.
- The `ask_model`, `search_pipeline`, and `capture_full_screen_command` Tauri commands now require a `conversationId: String` argument (and `ask_model` additionally requires `isFirstTurn: bool` and `slashCommand: Option<String>`). The frontend's `useModel` hook generates a stable trace id per session and threads it transparently. External callers that invoked these commands directly must update their `invoke()` calls. A new fire-and-forget `record_conversation_end` command lets the frontend signal end-of-conversation (used by `useModel.reset()` and `useModel.loadMessages()`) so the chat-domain trace file gets a clean closing line.
- **BREAKING**: Renamed the `[model]` section in `config.toml` to `[inference]` and reshaped it from a single `ollama_url` string into the providers schema described above. There is no backward-compatibility shim for the section name: if you had a custom `[model]` section, rename it to `[inference]` after upgrading; a flat `ollama_url` inside `[inference]` is migrated automatically.
- Active model selection is now strictly Option-typed end to end: when nothing is installed and nothing is persisted, Thuki refuses to dispatch requests and surfaces a "Pick a model" prompt instead of falling back to a hardcoded slug. The previous `DEFAULT_MODEL_NAME` constant has been removed.

## [0.15.7](https://github.com/quiet-node/thuki/compare/v0.15.6...v0.15.7) (2026-07-02)


### Bug Fixes

* cancel in-flight generation when starting a new session ([#289](https://github.com/quiet-node/thuki/issues/289)) ([da432fa](https://github.com/quiet-node/thuki/commit/da432fa2349d411887d7263fa4e8b4ed71b8a0d6))
* skip false starting-up copy when engine is already loaded ([#291](https://github.com/quiet-node/thuki/issues/291)) ([047824c](https://github.com/quiet-node/thuki/commit/047824ce2a8db529665369b98a60dcc017b544f3))

## [0.15.6](https://github.com/quiet-node/thuki/compare/v0.15.5...v0.15.6) (2026-07-01)


### Bug Fixes

* show cold-start loading label during engine warmup ([#287](https://github.com/quiet-node/thuki/issues/287)) ([bf75c42](https://github.com/quiet-node/thuki/commit/bf75c4275476009599e7f63162270ec9db10ea46))

## [0.15.5](https://github.com/quiet-node/thuki/compare/v0.15.4...v0.15.5) (2026-07-01)


### Bug Fixes

* **context:** detect clipboard fallback copy by content, not by change ([#285](https://github.com/quiet-node/thuki/issues/285)) ([89ae62a](https://github.com/quiet-node/thuki/commit/89ae62a33a633e1555dcc1e0171f042fad02765e))

## [0.15.4](https://github.com/quiet-node/thuki/compare/v0.15.3...v0.15.4) (2026-06-30)


### Bug Fixes

* **windows:** make Settings and What's New standard single-Space windows ([#283](https://github.com/quiet-node/thuki/issues/283)) ([0196729](https://github.com/quiet-node/thuki/commit/0196729c8d3ef31a9ff188f692411fb409ca9da6))

## [0.15.3](https://github.com/quiet-node/thuki/compare/v0.15.2...v0.15.3) (2026-06-30)


### Bug Fixes

* **ui:** hide Dock icon on Settings close and refocus without recentering ([#280](https://github.com/quiet-node/thuki/issues/280)) ([ff6e44c](https://github.com/quiet-node/thuki/commit/ff6e44cc5a77e840aa806b4a217d344bf4bbdfbf))
* **window:** hide Dock icon on Settings close and refocus without recentering ([ff6e44c](https://github.com/quiet-node/thuki/commit/ff6e44cc5a77e840aa806b4a217d344bf4bbdfbf))

## [0.15.2](https://github.com/quiet-node/thuki/compare/v0.15.1...v0.15.2) (2026-06-30)


### Features

* **models:** clickable Browse-all quant filenames + non-blocking model delete ([#271](https://github.com/quiet-node/thuki/issues/271)) ([b68c663](https://github.com/quiet-node/thuki/commit/b68c66329ad872da5593fb60c085d68cc9018497))


### Bug Fixes

* **window:** dock icon + normal layering for Settings and onboarding ([#273](https://github.com/quiet-node/thuki/issues/273)) ([a439e5e](https://github.com/quiet-node/thuki/commit/a439e5e10c04b5678ac8f31e55a572ed59addac7))


### Miscellaneous Chores

* release 0.15.2 ([0ddbf68](https://github.com/quiet-node/thuki/commit/0ddbf685935d9ed97a8f12f6bf2b6015d6c369ef))

## [0.15.1](https://github.com/quiet-node/thuki/compare/v0.15.0...v0.15.1) (2026-06-30)


### Bug Fixes

* **ci:** cap engine build parallelism by RAM to avoid OOM stalls ([#270](https://github.com/quiet-node/thuki/issues/270)) ([6904cb2](https://github.com/quiet-node/thuki/commit/6904cb28506782050db00793187492ff31e72274))
* **ci:** cap engine build parallelism by RAM to avoid OOM stalls on CI runners ([6904cb2](https://github.com/quiet-node/thuki/commit/6904cb28506782050db00793187492ff31e72274))
* **ci:** fetch llama-server sidecar before lint in release job ([0824f2c](https://github.com/quiet-node/thuki/commit/0824f2cabb9a34d0ad669e14a0c328008e10e2f2))
* **ci:** fetch llama-server sidecar before lint in release publish job ([#261](https://github.com/quiet-node/thuki/issues/261)) ([0824f2c](https://github.com/quiet-node/thuki/commit/0824f2cabb9a34d0ad669e14a0c328008e10e2f2))
* **downloads:** RAM-fit starter swap, non-blocking sends, cross-window pause/discard sync ([#269](https://github.com/quiet-node/thuki/issues/269)) ([faf681c](https://github.com/quiet-node/thuki/commit/faf681c505781c699afa70ad711149737a451fa1))
* **engine:** support macOS 13.4+ for the built-in engine ([#266](https://github.com/quiet-node/thuki/issues/266)) ([8226773](https://github.com/quiet-node/thuki/commit/8226773290244cf2c2036387aafdc8e0a8ee536f))
* **models:** update UI for multi-part (split) GGUF models ([#267](https://github.com/quiet-node/thuki/issues/267)) ([3c0c1de](https://github.com/quiet-node/thuki/commit/3c0c1de0bdb3262d15df32a2527d536fa594877a))

## [0.15.0](https://github.com/quiet-node/thuki/compare/v0.14.3...v0.15.0) (2026-06-29)


### Features

* built-in engine onboarding, model downloads, Settings providers, and default flip ([#219](https://github.com/quiet-node/thuki/issues/219)) ([171a6a3](https://github.com/quiet-node/thuki/commit/171a6a3dc2212f3a8079b0cee727b2c98b6fc0b3))
* bundled engine runner and model library ([#217](https://github.com/quiet-node/thuki/issues/217)) ([faa82ca](https://github.com/quiet-node/thuki/commit/faa82caba2416327a2992153f8a6bcf2237ea6e2))
* **models:** in-app model library with Discover browser and Staff Picks ([#237](https://github.com/quiet-node/thuki/issues/237)) ([23b32ef](https://github.com/quiet-node/thuki/commit/23b32efc13b615c4d0763e1bc7a09d3bd557f9b1))
* **models:** sync live download progress across windows ([#250](https://github.com/quiet-node/thuki/issues/250)) ([4e14d66](https://github.com/quiet-node/thuki/commit/4e14d6654dc835c3a7380d94368bf37e2e16bbc3))
* **onboarding:** built-in engine upgrade announcement and onboarding/picker polish ([#251](https://github.com/quiet-node/thuki/issues/251)) ([cfc0c62](https://github.com/quiet-node/thuki/commit/cfc0c62e246d4ce1716b95cf0d5c0cf1c3ef6622))
* **onboarding:** optional email capture to help shape Thuki ([#254](https://github.com/quiet-node/thuki/issues/254)) ([b5c6f5c](https://github.com/quiet-node/thuki/commit/b5c6f5cfc98db7341fbaad78395579ea51fcbe8e))
* **onboarding:** surface-aware download strip and model-library intro fact ([#252](https://github.com/quiet-node/thuki/issues/252)) ([f6240bd](https://github.com/quiet-node/thuki/commit/f6240bdeba37c99f7cc5f3745e8992c13b872eb9))
* OpenAI-compatible /v1 client and provider routing ([#218](https://github.com/quiet-node/thuki/issues/218)) ([25fe634](https://github.com/quiet-node/thuki/commit/25fe63480260eb951c01bff632516d379e0d4ab7))
* restore the Providers setting reverted by [#208](https://github.com/quiet-node/thuki/issues/208) ([#215](https://github.com/quiet-node/thuki/issues/215)) ([5a5310f](https://github.com/quiet-node/thuki/commit/5a5310f0ed2668110510bf137d1366061c195ae0))
* **settings:** subtle model-name links with hover reveal ([#255](https://github.com/quiet-node/thuki/issues/255)) ([bcfaa78](https://github.com/quiet-node/thuki/commit/bcfaa781888277fc54d6a56f0740689d19ad386b))
* **settings:** subtle model-name links with hover reveal; italic founder name ([bcfaa78](https://github.com/quiet-node/thuki/commit/bcfaa781888277fc54d6a56f0740689d19ad386b))


### Bug Fixes

* ad-hoc sign local macOS builds so TCC grants apply ([#229](https://github.com/quiet-node/thuki/issues/229)) ([5c3a2df](https://github.com/quiet-node/thuki/commit/5c3a2dfd4f65b1cfce36a447ad0f9a86a34721dd))
* fetch llama-server sidecar before lint in nightly release ([#235](https://github.com/quiet-node/thuki/issues/235)) ([d039999](https://github.com/quiet-node/thuki/commit/d03999935eebbe84ec37312976f638d4559fedd8))
* gate overlay activation during onboarding to prevent window collapse ([#233](https://github.com/quiet-node/thuki/issues/233)) ([b111b1c](https://github.com/quiet-node/thuki/commit/b111b1c36210a932d3489e4d7748f4cd953a59e2))
* onboarding permission loop on local macOS builds ([#230](https://github.com/quiet-node/thuki/issues/230)) ([5c3a2df](https://github.com/quiet-node/thuki/commit/5c3a2dfd4f65b1cfce36a447ad0f9a86a34721dd))
* persist onboarding progress so relaunch can't bounce permissions ([#229](https://github.com/quiet-node/thuki/issues/229)) ([a0ecbbb](https://github.com/quiet-node/thuki/commit/a0ecbbb43e3b650a618816ed4cf352dd8d153b34))
* **prompt:** stop model from emitting slash commands as replies ([#243](https://github.com/quiet-node/thuki/issues/243)) ([fa08c8c](https://github.com/quiet-node/thuki/commit/fa08c8c712efe1464d6ef93713fe0e555a24a905))
* regain overlay focus after defocus via hover-activate tracking area ([#234](https://github.com/quiet-node/thuki/issues/234)) ([dec7535](https://github.com/quiet-node/thuki/commit/dec75354d4c779ef5a7b5c4d42cdae1325ad82e7))
* trim the system prompt and refresh non-customized prompts on load ([#239](https://github.com/quiet-node/thuki/issues/239)) ([395c77b](https://github.com/quiet-node/thuki/commit/395c77ba624f097107eec354904cad0a587d3e29))


### Reverts

* restore onboarding permission-revocation detection ([#231](https://github.com/quiet-node/thuki/issues/231)) ([#232](https://github.com/quiet-node/thuki/issues/232)) ([1f8a121](https://github.com/quiet-node/thuki/commit/1f8a12182f39309c646e41ccd2eec8bdd9deaee1))

## [0.14.3](https://github.com/quiet-node/thuki/compare/v0.14.2...v0.14.3) (2026-06-09)


### Bug Fixes

* emit terminal Done when the model stream ends without a done marker ([#212](https://github.com/quiet-node/thuki/issues/212)) ([df09b2a](https://github.com/quiet-node/thuki/commit/df09b2a029b34cf2e0241790fc0f1618c0ac55be))

## [0.14.2](https://github.com/quiet-node/thuki/compare/v0.14.1...v0.14.2) (2026-06-08)


### Bug Fixes

* make /rewrite preserve formatting, structure, and verbatim content ([#207](https://github.com/quiet-node/thuki/issues/207)) ([d3d2edb](https://github.com/quiet-node/thuki/commit/d3d2edb36728dcb5664711f3034bf70cb749aec0))

## [0.14.1](https://github.com/quiet-node/thuki/compare/v0.14.0...v0.14.1) (2026-06-07)


### Bug Fixes

* show all versions between installed and latest in What's New ([#203](https://github.com/quiet-node/thuki/issues/203)) ([792b098](https://github.com/quiet-node/thuki/commit/792b098e6e2c4edef2f2525a3b98f8721787385b))

## [0.14.0](https://github.com/quiet-node/thuki/compare/v0.13.1...v0.14.0) (2026-06-07)


### Features

* allow drafting messages while response is streaming in AskBarView ([#200](https://github.com/quiet-node/thuki/issues/200)) ([108e1eb](https://github.com/quiet-node/thuki/commit/108e1eb3e367778b24e41b6e1be096647eadbf6c))
* migrate AskBar input to Lexical to fix WKWebView caret drift ([#202](https://github.com/quiet-node/thuki/issues/202)) ([adafe47](https://github.com/quiet-node/thuki/commit/adafe4729f55e898bba46607d527f629b60bace9))
* pre-load conversation list before opening ask-bar history drawer ([#199](https://github.com/quiet-node/thuki/issues/199)) ([5ac73e0](https://github.com/quiet-node/thuki/commit/5ac73e0bdbcfedfef454842671b70d275d77da70))
* redesign /rewrite for a natural, casual voice ([#201](https://github.com/quiet-node/thuki/issues/201)) ([c66519a](https://github.com/quiet-node/thuki/commit/c66519a71d3557e73b9f3f2c1e027435a8cca232))
* write /rewrite and /refine results back into the source app ([#197](https://github.com/quiet-node/thuki/issues/197)) ([03a8fce](https://github.com/quiet-node/thuki/commit/03a8fce0c2c136a2e98638315b4cebb298e1f84d))

## [0.13.1](https://github.com/quiet-node/thuki/compare/v0.13.0...v0.13.1) (2026-05-26)


### Bug Fixes

* **screenshot:** capture display containing Thuki window on multi-monitor setups ([#191](https://github.com/quiet-node/thuki/issues/191)) ([611dff1](https://github.com/quiet-node/thuki/commit/611dff1d2b57a1bac709cbc227f0f98615b3d7be))

## [0.13.0](https://github.com/quiet-node/thuki/compare/v0.12.0...v0.13.0) (2026-05-25)


### Features

* export chat session to Markdown and clipboard ([#189](https://github.com/quiet-node/thuki/issues/189)) ([e8eeed0](https://github.com/quiet-node/thuki/commit/e8eeed05a61babd9e4efd190afb7d97f46f99658))

## [0.12.0](https://github.com/quiet-node/thuki/compare/v0.11.3...v0.12.0) (2026-05-22)


### Features

* **minimize:** collapse the chat to a floating mascot with edge-aware restore ([#187](https://github.com/quiet-node/thuki/issues/187)) ([23507c0](https://github.com/quiet-node/thuki/commit/23507c0d111ff879cde30e67687726e9f177a08f))

## [0.11.3](https://github.com/quiet-node/thuki/compare/v0.11.2...v0.11.3) (2026-05-18)


### Bug Fixes

* **updater:** embed full release changelog in updater manifest ([#182](https://github.com/quiet-node/thuki/issues/182)) ([6462266](https://github.com/quiet-node/thuki/commit/646226691fb8ebb9912d38327909d2f328d67d7b))

## [0.11.2](https://github.com/quiet-node/thuki/compare/v0.11.1...v0.11.2) (2026-05-18)


### Bug Fixes

* **math:** escape currency dollars so they aren't parsed as LaTeX math ([#180](https://github.com/quiet-node/thuki/issues/180)) ([90faee1](https://github.com/quiet-node/thuki/commit/90faee18535e58eb25df6f5310f07ccb1da4a9d3))

## [0.11.1](https://github.com/quiet-node/thuki/compare/v0.11.0...v0.11.1) (2026-05-16)


### UI

* **updater:** redesign the What's New window to match the Settings panel ([#177](https://github.com/quiet-node/thuki/issues/177)) ([9f80719](https://github.com/quiet-node/thuki/commit/9f80719234913e423d7025cc4a976d6b823c0459))

## [0.11.0](https://github.com/quiet-node/thuki/compare/v0.10.0...v0.11.0) (2026-05-16)


### Features

* **updater:** What's New update window with explicit actions; open Settings on current Space ([#174](https://github.com/quiet-node/thuki/issues/174)) ([0243c4b](https://github.com/quiet-node/thuki/commit/0243c4b568980b6b35441cf55065ac2d0993c7d4))

## [0.10.0](https://github.com/quiet-node/thuki/compare/v0.9.1...v0.10.0) (2026-05-15)


### Features

* **settings:** user-configurable typography controls for chat and input ([#172](https://github.com/quiet-node/thuki/issues/172)) ([03e523c](https://github.com/quiet-node/thuki/commit/03e523ce98d77ae5ee435602c6102a8aed542163))

## [0.9.1](https://github.com/quiet-node/thuki/compare/v0.9.0...v0.9.1) (2026-05-13)


### Bug Fixes

* **ui:** replace Inter and Source Serif 4 with Nunito as sole typeface ([#167](https://github.com/quiet-node/thuki/issues/167)) ([fec2c49](https://github.com/quiet-node/thuki/commit/fec2c494ef893b29fe36692bb3f672b6b21574f7))

## [0.9.0](https://github.com/quiet-node/thuki/compare/v0.8.5...v0.9.0) (2026-05-12)


### Features

* **commands:** add /explain slash command with /screen and image support ([#159](https://github.com/quiet-node/thuki/issues/159)) ([b78e9b3](https://github.com/quiet-node/thuki/commit/b78e9b3664cf8f8d1031f7b84778f9c563ed1c3f))
* **commands:** add /extract slash command with Vision OCR text extraction ([#160](https://github.com/quiet-node/thuki/issues/160)) ([aafe2fc](https://github.com/quiet-node/thuki/commit/aafe2fc2054615639a7a88803b18c6947d749edd))
* **commands:** unified slash command dispatch + OCR utility commands ([#164](https://github.com/quiet-node/thuki/issues/164)) ([22fc98f](https://github.com/quiet-node/thuki/commit/22fc98fb021fafec64182882eed3b7a8133e73e5))
* **markdown:** add KaTeX math rendering via Streamdown plugin API ([#156](https://github.com/quiet-node/thuki/issues/156)) ([579a93b](https://github.com/quiet-node/thuki/commit/579a93bef0c7d513adf8550cb1d8a1ff41b580c3))


### Bug Fixes

* **config:** restore default system prompt on upgrade for uncustomized configs ([#158](https://github.com/quiet-node/thuki/issues/158)) ([43e0386](https://github.com/quiet-node/thuki/commit/43e03863082cc59c4340ab9cd2d313aaeefe4f62))

## [0.8.5](https://github.com/quiet-node/thuki/compare/v0.8.4...v0.8.5) (2026-05-08)


### Bug Fixes

* **permissions:** clear stale TCC entries on upgrade and grant click ([#153](https://github.com/quiet-node/thuki/issues/153)) ([f6d9ca2](https://github.com/quiet-node/thuki/commit/f6d9ca2c9e8ffce8299be633f6a7d4338e990841))

## [0.8.4](https://github.com/quiet-node/thuki/compare/v0.8.3...v0.8.4) (2026-05-07)


### Bug Fixes

* **updater:** relaunch after TCC reset so System Settings can re-register Thuki ([#151](https://github.com/quiet-node/thuki/issues/151)) ([27dc003](https://github.com/quiet-node/thuki/commit/27dc0031b06da23dcc72de8183f59cb5e790ab2b))
* **updater:** relaunch after TCC reset to refresh tccd PID tracking ([27dc003](https://github.com/quiet-node/thuki/commit/27dc0031b06da23dcc72de8183f59cb5e790ab2b))

## [0.8.3](https://github.com/quiet-node/thuki/compare/v0.8.2...v0.8.3) (2026-05-07)


### Bug Fixes

* **updater:** clear snoozes when a new version becomes available ([#149](https://github.com/quiet-node/thuki/issues/149)) ([c672409](https://github.com/quiet-node/thuki/commit/c6724095663b51ce2cce38b6410d668a53c10f40))

## [0.8.2](https://github.com/quiet-node/thuki/compare/v0.8.1...v0.8.2) (2026-05-07)


### Bug Fixes

* **updater:** timestamp on errors and footer in chat mode ([#147](https://github.com/quiet-node/thuki/issues/147)) ([92a2e15](https://github.com/quiet-node/thuki/commit/92a2e151e5437868b48d56470b36192596a8f890))

## [0.8.1](https://github.com/quiet-node/thuki/compare/v0.8.0...v0.8.1) (2026-05-07)


### Bug Fixes

* **settings:** redesign About Updates as hero card with check animation ([#145](https://github.com/quiet-node/thuki/issues/145)) ([b4190e1](https://github.com/quiet-node/thuki/commit/b4190e1958b72dd83334aa6f48430dcee644547a))

## [0.8.0](https://github.com/quiet-node/thuki/compare/v0.7.1...v0.8.0) (2026-05-07)


### Features

* **trace:** unified per-conversation forensic recorder for chat + search ([#139](https://github.com/quiet-node/thuki/issues/139)) ([76f9180](https://github.com/quiet-node/thuki/commit/76f91802ac248e5acd210721f20dc233654b5d9d))
* **updater:** in-app auto-update via signed GitHub releases ([#144](https://github.com/quiet-node/thuki/issues/144)) ([7e5b833](https://github.com/quiet-node/thuki/commit/7e5b833eed2aee45c1614aa4b36b1b8671b0e152))


### Bug Fixes

* **ui:** adopt Source Serif 4 for AI prose reading register ([#140](https://github.com/quiet-node/thuki/issues/140)) ([5adc86d](https://github.com/quiet-node/thuki/commit/5adc86dfa1ad91b5358df1b381bcca7c0b9d6e10))

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
