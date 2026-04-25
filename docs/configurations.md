# Configurations

Thuki reads its runtime configuration from a single TOML file located at:

```
~/Library/Application Support/com.quietnode.thuki/config.toml
```

The file is created automatically the first time the app launches. You can edit it with any text editor; changes take effect on the next launch. A future Settings panel will let you make the same changes from inside the app, writing to the same file.

## First launch

You do not need to do anything. Thuki writes a default `config.toml` on first run with every field set to a sensible value.

If the directory cannot be written (disk full, permission denied, read-only filesystem), Thuki shows a native alert with the specific error and exits. This is a macOS-level setup problem; Thuki cannot repair it on your behalf.

## Editing

Open the file, change a value, save, relaunch Thuki.

```bash
# Opens the file in your default TextEdit-like editor
open ~/Library/Application\ Support/com.quietnode.thuki/config.toml
```

### Example

```toml
[model]
# First entry is the ACTIVE model used for all inference.
# Reorder the list to switch models (requires app restart in this release).
# Run `ollama pull <model>` before adding a model you haven't used.
available = ["gemma4:e2b", "gemma4:e4b"]
ollama_url = "http://127.0.0.1:11434"

[prompt]
# Leave empty to use the built-in secretary persona.
# Thuki always appends the generated slash-command appendix at runtime,
# whether or not this field is set, so slash commands keep working.
system = ""

[window]
overlay_width = 600
collapsed_height = 80
max_chat_height = 648
hide_commit_delay_ms = 350

[quote]
max_display_lines = 4
max_display_chars = 300
max_context_length = 4096

[search]
# URLs of the local sandbox services. Match the bindings in
# `sandbox/docker-compose.yml`. Override only if you run SearXNG or the
# reader sidecar on a different host or port.
searxng_url = "http://127.0.0.1:25017"
reader_url = "http://127.0.0.1:25018"
# Pipeline tuning: trade quality against latency.
max_iterations = 3
top_k_urls = 10
# Per-stage timeouts in seconds.
search_timeout_s = 20
reader_per_url_timeout_s = 10
reader_batch_timeout_s = 30
judge_timeout_s = 30
router_timeout_s = 45
```

## Reading the reference tables

Every domain below is shown as a single table that lists **all** constants Thuki uses in that area: both the ones you can tune in `config.toml` and the ones baked in at compile time. The columns are:

- **Constant**: the TOML key (tunable) or Rust/TypeScript identifier (baked-in).
- **Default**: the value Thuki ships with.
- **Tunable?**: `Yes` if editable via `config.toml`, `No` if compiled in.
- **Why not tunable**: only filled for baked-in constants; explains why it is locked.
- **Bounds**: only filled for tunable numerics; values outside are clamped to defaults with a stderr warning.
- **Description**: what the constant controls.

## Reference

### `[model]`

Active model and the Ollama endpoint Thuki talks to.

| Constant     | Default                    | Tunable? | Why not tunable | Bounds         | Description                                                                                 |
| :----------- | :------------------------- | :------- | :-------------- | :------------- | :------------------------------------------------------------------------------------------ |
| `available`  | `["gemma4:e2b"]`           | Yes      | —               | non-empty list | Ordered list of Ollama model names. **First entry is the active model.** Reorder to switch. |
| `ollama_url` | `"http://127.0.0.1:11434"` | Yes      | —               | non-empty URL  | HTTP base URL of the local Ollama instance.                                                 |

If the active model has not been pulled, the next request surfaces a "Model not found" error with the exact `ollama pull <name>` command to run.

### `[prompt]`

The system prompt Thuki sends with every conversation.

| Constant                        | Default                                | Tunable? | Why not tunable                                                                                                                                   | Bounds     | Description                                                        |
| :------------------------------ | :------------------------------------- | :------- | :------------------------------------------------------------------------------------------------------------------------------------------------ | :--------- | :----------------------------------------------------------------- |
| `system`                        | `""`                                   | Yes      | —                                                                                                                                                 | any string | User persona prompt. Empty = use built-in Thuki secretary persona. |
| `DEFAULT_SYSTEM_PROMPT_BASE`    | `prompts/system_prompt.txt`            | No       | Built-in fallback; the user-editable override is `system` above.                                                                                  | —          | Secretary persona used when `system` is empty.                     |
| `SLASH_COMMAND_PROMPT_APPENDIX` | `prompts/generated/slash_commands.txt` | No       | Auto-generated from the slash-command registry at build time; editing it by hand would desynchronize model instructions from the actual commands. | —          | Appended to every prompt so slash-command knowledge is never lost. |

### `[window]`

Overlay panel geometry.

| Constant               | Default | Tunable? | Why not tunable | Bounds            | Description                                                                |
| :--------------------- | :------ | :------- | :-------------- | :---------------- | :------------------------------------------------------------------------- |
| `overlay_width`        | `600.0` | Yes      | —               | `[200.0, 2000.0]` | Logical width of the overlay panel.                                        |
| `collapsed_height`     | `80.0`  | Yes      | —               | `[40.0, 400.0]`   | Height of the AskBar (collapsed) state.                                    |
| `max_chat_height`      | `648.0` | Yes      | —               | `[200.0, 2000.0]` | Upper bound on the expanded chat window.                                   |
| `hide_commit_delay_ms` | `350`   | Yes      | —               | `[0, 5000]`       | Delay (ms) before the native window hides after the exit animation starts. |

### `[quote]`

Selected-text quote preview and context forwarding.

| Constant             | Default | Tunable? | Why not tunable | Bounds       | Description                                              |
| :------------------- | :------ | :------- | :-------------- | :----------- | :------------------------------------------------------- |
| `max_display_lines`  | `4`     | Yes      | —               | `[1, 100]`   | Maximum lines shown in the quote preview.                |
| `max_display_chars`  | `300`   | Yes      | —               | `[1, 10000]` | Maximum characters shown in the quote preview.           |
| `max_context_length` | `4096`  | Yes      | —               | `[1, 65536]` | Maximum characters of selected text forwarded to Ollama. |

### `[search]`

Agentic `/search` pipeline: sandbox endpoints, iteration/top-K budgets, per-stage timeouts, and the pipeline-internal shape constants the prompts depend on.

URLs must include scheme, host, and port, with no path; Thuki appends `/search` and `/extract` automatically. Empty strings are replaced with the compiled defaults at load time, and out-of-bounds numerics are clamped (a warning is logged to stderr).

For security, both URLs default to loopback (`127.0.0.1`) and are intended to stay there. Pointing them at a remote host disables the sandbox's network isolation guarantees.

| Constant                        | Default                    | Tunable? | Why not tunable                                                                                                                              | Bounds        | Description                                                                                                                                                   |
| :------------------------------ | :------------------------- | :------- | :------------------------------------------------------------------------------------------------------------------------------------------- | :------------ | :------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `searxng_url`                   | `"http://127.0.0.1:25017"` | Yes      | —                                                                                                                                            | non-empty URL | Base URL of the SearXNG instance.                                                                                                                             |
| `reader_url`                    | `"http://127.0.0.1:25018"` | Yes      | —                                                                                                                                            | non-empty URL | Base URL of the reader/extractor sidecar.                                                                                                                     |
| `max_iterations`                | `3`                        | Yes      | —                                                                                                                                            | `[1, 10]`     | Max search-refine rounds before the pipeline gives up.                                                                                                        |
| `top_k_urls`                    | `10`                       | Yes      | —                                                                                                                                            | `[1, 20]`     | URLs forwarded to the reader after reranking.                                                                                                                 |
| `search_timeout_s`              | `20`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | Seconds before a SearXNG query is abandoned.                                                                                                                  |
| `reader_per_url_timeout_s`      | `10`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | Seconds allowed for a single URL fetch in the reader.                                                                                                         |
| `reader_batch_timeout_s`        | `30`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | Seconds allowed for the full parallel reader batch. Must exceed `reader_per_url_timeout_s`; the loader corrects violations to `reader_per_url_timeout_s + 5`. |
| `judge_timeout_s`               | `30`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | Seconds before the judge LLM call is abandoned.                                                                                                               |
| `router_timeout_s`              | `45`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | Seconds before the router LLM call is abandoned.                                                                                                              |
| `GAP_QUERIES_PER_ROUND`         | `3`                        | No       | Drives the judge-normalization cap and the prompt structure; changing it silently alters output quality rather than producing a clear error. | —             | Gap-filling queries generated per iteration round.                                                                                                            |
| `CHUNK_TOKEN_SIZE`              | `500`                      | No       | Downstream synthesis prompts assume this exact chunk size; rerank scoring is calibrated to it.                                               | —             | Approximate token budget per retrieved chunk.                                                                                                                 |
| `TOP_K_CHUNKS`                  | `8`                        | No       | Coupled to the synthesis prompt's context budget; larger values overflow the model window.                                                   | —             | Highest-scoring chunks forwarded to synthesis.                                                                                                                |
| `DEFAULT_READER_RETRY_DELAY_MS` | `500`                      | No       | Balances pressure on the sandbox reader against perceived responsiveness; no user signal that it needs to vary.                              | —             | Milliseconds before retrying a failed reader fetch.                                                                                                           |

### `[activation]` (not in TOML)

Double-tap Control hotkey and the Accessibility permission poller. The entire domain is compiled in: the CGEventTap callback runs on a thread that cannot read Tauri managed state, so exposing these through `config.toml` would require a redesign of the event-tap plumbing.

| Constant                   | Default  | Tunable? | Why not tunable                                                                                                                           | Bounds | Description                                                                          |
| :------------------------- | :------- | :------- | :---------------------------------------------------------------------------------------------------------------------------------------- | :----- | :----------------------------------------------------------------------------------- |
| `ACTIVATION_WINDOW`        | `400 ms` | No       | Event-tap callback cannot read Tauri managed state; would require a redesign to expose. No user has reported needing a different cadence. | —      | Max time between the two Control taps that counts as an activation.                  |
| `ACTIVATION_COOLDOWN`      | `600 ms` | No       | Same as above.                                                                                                                            | —      | Minimum interval between successive activations; prevents accidental double-toggles. |
| `KC_PRIMARY_L`             | `0x3b`   | No       | macOS hardware key code for left Control. Not user-meaningful; wrong value would brick activation.                                        | —      | Left Control keycode.                                                                |
| `KC_PRIMARY_R`             | `0x3e`   | No       | macOS hardware key code for right Control. Not user-meaningful; wrong value would brick activation.                                       | —      | Right Control keycode.                                                               |
| `MAX_PERMISSION_ATTEMPTS`  | `6`      | No       | Internal retry budget for the Accessibility prompt; no user-facing reason to tune.                                                        | —      | How many times the permission poller retries.                                        |
| `PERMISSION_POLL_INTERVAL` | `5 s`    | No       | Same as above.                                                                                                                            | —      | Delay between permission-check cycles.                                               |

### `[vision]` (not in TOML)

Image attachments for multimodal requests. Limits are dictated by the Ollama vision input protocol and JPEG quality tradeoffs, so none are exposed for tuning.

| Constant                 | Default   | Tunable? | Why not tunable                                                                                                          | Bounds | Description                                                 |
| :----------------------- | :-------- | :------- | :----------------------------------------------------------------------------------------------------------------------- | :----- | :---------------------------------------------------------- |
| `MAX_IMAGES_PER_MESSAGE` | `4`       | No       | Protocol cap: 3 manual attachments + 1 `/screen` capture. Larger values make requests fail further downstream in Ollama. | —      | Maximum images per outgoing message.                        |
| `MAX_IMAGE_SIZE_BYTES`   | `30 MiB`  | No       | Frontend rejection threshold aligned with Ollama's practical decode ceiling.                                             | —      | Per-image upload size limit.                                |
| `MAX_DIMENSION`          | `1920 px` | No       | Downscale target that balances vision-model accuracy against payload size.                                               | —      | Maximum width/height for stored images (aspect-preserving). |
| `JPEG_QUALITY`           | `85`      | No       | Balances file size against visual fidelity for vision models; changes would invalidate historical saved images.          | —      | JPEG encoder quality (1–100).                               |

### `[history]` (not in TOML)

Conversation history panel UX.

| Constant             | Default | Tunable? | Why not tunable                                                                                                                              | Bounds | Description                                                                   |
| :------------------- | :------ | :------- | :------------------------------------------------------------------------------------------------------------------------------------------- | :----- | :---------------------------------------------------------------------------- |
| `SEARCH_DEBOUNCE_MS` | `200`   | No       | UX tuning; no meaningful user signal for changing this. Raising it makes search feel sluggish; lowering it wastes cycles on every keystroke. | —      | Milliseconds to wait after the last keystroke before firing a history search. |

## What happens on bad input

Thuki prefers to keep the app running with a usable configuration rather than fail noisily.

- **Missing file**: defaults written, app launches normally.
- **Missing fields**: filled in from compiled defaults; your other customizations stay.
- **Empty or whitespace-only strings**: replaced with compiled defaults at load time.
- **Out-of-bounds numeric values**: reset to compiled defaults; a warning is logged to stderr (visible via `Console.app`).
- **Unparseable TOML**: the file is renamed to `config.toml.corrupt-<unix_timestamp>` and a fresh defaults file is written. The old file is preserved so you can inspect or restore it by hand.

## Dev-time `.env` files

Thuki no longer reads `.env` files. Both `.env` and `.env.example` have been removed and the `dotenvy` dependency has been dropped. If you still have a local `.env` from an older checkout, it is ignored; you can delete it.
