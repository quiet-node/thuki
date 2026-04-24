# Configurations

Thuki reads its runtime configuration from a single TOML file located at:

```
~/Library/Application Support/com.quietnode.thuki/config.toml
```

The file is created automatically the first time the app launches. You can edit it with any text editor; changes take effect on the next launch. A future Settings panel will let you make the same changes from inside the app, writing to the same file.

## First launch

You do not need to do anything. Thuki writes a default `config.toml` on first run with every field set to a sensible value and a `schema_version = 1` marker.

If the directory cannot be written (disk full, permission denied, read-only filesystem), Thuki shows a native alert with the specific error and exits. This is a macOS-level setup problem; Thuki cannot repair it on your behalf.

## Editing

Open the file, change a value, save, relaunch Thuki.

```bash
# Opens the file in your default TextEdit-like editor
open ~/Library/Application\ Support/com.quietnode.thuki/config.toml
```

### Example

```toml
schema_version = 1

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
# Per-stage timeouts in seconds. Increase on slow networks or slow local
# hardware; decrease only if you know your sandbox is faster than these.
search_timeout_s = 20
reader_per_url_timeout_s = 10
reader_batch_timeout_s = 30
judge_timeout_s = 30
router_timeout_s = 45
```

## Reference

### `[model]`

| Field | Description | Default |
| :--- | :--- | :--- |
| `available` | Ordered list of Ollama model names Thuki may use. **The first entry is the active model for all inference.** Reorder the list to switch. | `["gemma4:e2b"]` |
| `ollama_url` | HTTP base URL of the local Ollama instance. | `"http://127.0.0.1:11434"` |

If you change `active` to a model that has not been pulled, the next request surfaces a "Model not found" error with the exact `ollama pull <name>` command to run.

### `[prompt]`

| Field | Description | Default |
| :--- | :--- | :--- |
| `system` | User-editable persona prompt prepended to every conversation. Leave empty (`""`) to use the built-in secretary persona. Thuki always appends its generated slash-command appendix separately, so slash command knowledge is never lost. | `""` |

### `[window]`

| Field | Description | Default | Bounds |
| :--- | :--- | :--- | :--- |
| `overlay_width` | Logical width of the overlay panel. | `600.0` | `[200.0, 2000.0]` |
| `collapsed_height` | Height of the AskBar (collapsed) state. | `80.0` | `[40.0, 400.0]` |
| `max_chat_height` | Upper bound on the expanded chat window. | `648.0` | `[200.0, 2000.0]` |
| `hide_commit_delay_ms` | Delay before the native window is hidden after the exit animation starts. | `350` | `[0, 5000]` |

### `[quote]`

Controls how selected-text quotes are shown in the AskBar preview and chat bubbles, and how much selected context is forwarded to Ollama.

| Field | Description | Default | Bounds |
| :--- | :--- | :--- | :--- |
| `max_display_lines` | Maximum number of lines shown in the quote preview. | `4` | `[1, 100]` |
| `max_display_chars` | Maximum total characters shown in the quote preview. | `300` | `[1, 10000]` |
| `max_context_length` | Maximum characters of selected text included in the prompt sent to Ollama. | `4096` | `[1, 65536]` |

### `[search]`

Controls the agentic `/search` pipeline: where to reach the local sandbox services, how many refinement rounds to attempt, and how long each network stage may run before being abandoned.

The defaults match the localhost-only ports bound by `sandbox/docker-compose.yml`. URLs must include the scheme, host, and port, with no path; Thuki appends the `/search` and `/extract` paths automatically. Empty strings are replaced with the compiled defaults at load time, and out-of-bounds numerics are clamped (a warning is logged to stderr).

| Field | Description | Default | Bounds |
| :--- | :--- | :--- | :--- |
| `searxng_url` | Base URL of the SearXNG instance. | `"http://127.0.0.1:25017"` | non-empty |
| `reader_url` | Base URL of the reader/extractor sidecar. | `"http://127.0.0.1:25018"` | non-empty |
| `max_iterations` | Maximum number of search-refine iterations before the pipeline gives up. | `3` | `[1, 10]` |
| `top_k_urls` | Number of top-ranked URLs forwarded to the reader after reranking. | `10` | `[1, 20]` |
| `search_timeout_s` | Seconds before a SearXNG query is abandoned. | `20` | `[1, 300]` |
| `reader_per_url_timeout_s` | Seconds allowed for a single URL fetch inside the reader. | `10` | `[1, 300]` |
| `reader_batch_timeout_s` | Seconds allowed for the full parallel reader batch. Must exceed `reader_per_url_timeout_s`; the loader corrects violations to `reader_per_url_timeout_s + 5`. | `30` | `[1, 300]` |
| `judge_timeout_s` | Seconds before the judge LLM call is abandoned. | `30` | `[1, 300]` |
| `router_timeout_s` | Seconds before the router LLM call is abandoned. | `45` | `[1, 300]` |

For security, both URLs default to loopback (`127.0.0.1`) and are intended to stay there. Pointing them at a remote host disables the sandbox's network isolation guarantees: the reader will fetch arbitrary pages on behalf of the LLM router output, which is an SSRF amplifier if the host has access to non-public networks.

## What happens on bad input

Thuki prefers to keep the app running with a usable configuration rather than fail noisily.

- **Missing file**: defaults written, app launches normally.
- **Missing fields**: filled in from compiled defaults; your other customizations stay.
- **Empty or whitespace-only strings**: replaced with compiled defaults at load time.
- **Out-of-bounds numeric values**: reset to compiled defaults; a warning is logged to stderr (visible via `Console.app`).
- **Unparseable TOML or unknown `schema_version`**: the file is renamed to `config.toml.corrupt-<unix_timestamp>` and a fresh defaults file is written. The old file is preserved so you can inspect or restore it by hand.

## What is NOT configurable (and why)

A few knobs that look configurable on the surface are intentionally kept out of `config.toml`:

- **Search-pipeline shape constants** (`GAP_QUERIES_PER_ROUND`, `CHUNK_TOKEN_SIZE`, `TOP_K_CHUNKS`, retry delays). Downstream prompt design and persisted metadata interpretation depend on these exact values; tuning them wrong produces subtle drift rather than a clear error. See `src-tauri/src/search/config.rs`. The user-tunable knobs (iterations, top-K URLs, timeouts, service URLs) are exposed in `[search]` above.
- **macOS key codes** (`0x3b`, `0x3e` for left and right Control). Not user-meaningful; wrong values would brick activation.
- **Activation timing** (400 ms double-tap window, 600 ms cooldown). These are compiled constants in `src-tauri/src/activator.rs`. Not yet exposed because the CGEventTap callback lives in a thread that cannot trivially read Tauri managed state, and no user has reported needing a different cadence. A future PR can promote these if the need appears.
- **Image limits** (4 images per message, 30 MiB per image). Protocol caps imposed by Ollama's vision input; a larger value just makes requests fail further downstream.
- **History search debounce** (200 ms). UX tuning; no meaningful user signal for changing this.

All of the above live as Rust or TypeScript constants. If a genuine need appears (a user reports the current value is wrong for their hardware or workflow), that value gets promoted into `config.toml` with a migration.

## Dev-time `.env` files

Thuki no longer reads `.env` files. Both `.env` and `.env.example` have been removed and the `dotenvy` dependency has been dropped. If you still have a local `.env` from an older checkout, it is ignored; you can delete it.
