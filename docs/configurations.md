# Configurations

Thuki reads its runtime configuration from a single TOML file located at:

```
~/Library/Application Support/com.quietnode.thuki/config.toml
```

The file is created automatically the first time the app launches. You can edit it with any text editor; changes take effect on the next launch. The in-app Settings panel (open it from the Thuki menu-bar icon) writes to this same file, so editing by hand and clicking through the panel are interchangeable.

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
[inference]
# The provider Thuki sends inference to. Defaults to the Built-in engine,
# the bundled llama.cpp server. Switch to Ollama anytime.
active_provider = "builtin"
# Context window size in tokens sent to the active provider with every request.
# For the built-in engine the value becomes `--ctx-size` when the llama-server
# process starts, so changing it restarts the engine. For Ollama, warmup and
# chat share this value so the same runner and its cached KV prefix for the
# system prompt are reused. Raise to fit longer conversations; lower to reduce
# GPU memory use. Valid range: 2048-1048576.
num_ctx = 16384
# Minutes of inactivity before Thuki releases the active model from memory.
# Applies to both local providers (the built-in engine and Ollama).
# 0 = use the provider's natural short default (~5 min): Ollama defers to its
#     own timer, the built-in engine applies its own ~5-minute timer.
# -1 = keep resident forever. Valid range: -1 or 0-1440.
keep_warm_inactivity_minutes = 0

# One block per provider. The built-in entry is always present. A provider's
# selected model lives on its own `model` field (empty until you pick one in
# the model picker).
[[inference.providers]]
id = "builtin"
kind = "builtin"
label = "Built-in"
model = ""

[[inference.providers]]
id = "ollama"
kind = "ollama"
label = "Ollama"
# Where Thuki reaches your Ollama server. Defaults to this Mac; point it at
# another machine to use Ollama running elsewhere (one server at a time).
base_url = "http://127.0.0.1:11434"
model = ""

[prompt]
# The full secretary persona prompt. Seeded on first run. Save changes via
# Settings, which marks the prompt customized so your edit is kept; a hand edit
# made directly here only survives if you also set system_customized = true.
# While system_customized is false the stored value is treated as a cached
# default and refreshed to the built-in prompt on the next load. Clearing the
# prompt via Settings sends only the slash-command appendix, which Thuki always
# appends at runtime so slash commands keep working.
system = "..."
system_customized = false

[window]
overlay_width = 600
max_chat_height = 648
max_images = 3
text_base_px = 15.0
text_line_height = 1.5
text_letter_spacing_px = 0.0
text_font_weight = 500

[quote]
max_display_lines = 4
max_display_chars = 300
max_context_length = 4096

[behavior]
# Write /rewrite and /refine results straight back into the source app,
# replacing your selection, without clicking the in-chat Replace button.
auto_replace = false
# Dismiss the Thuki overlay after a /rewrite or /refine result is replaced
# back into the source app (manual Replace click or auto-replace).
auto_close = false

[search]
# URLs of the local sandbox services. Match the bindings in
# `sandbox/docker-compose.yml`. Override only if you run SearXNG or the
# reader sidecar on a different host or port.
searxng_url = "http://127.0.0.1:25017"
reader_url = "http://127.0.0.1:25018"
# Pipeline tuning: trade quality against latency.
max_iterations = 3
top_k_urls = 10
searxng_max_results = 10
# Per-stage timeouts in seconds.
search_timeout_s = 20
reader_per_url_timeout_s = 10
reader_batch_timeout_s = 30
judge_timeout_s = 30
router_timeout_s = 45

[debug]
# Records every chat conversation and /search session to disk for later inspection.
trace_enabled = false

[updater]
# Poll for new Thuki releases at startup and on a recurring interval.
auto_check = true
# Hours between background checks. Bound to 1..168.
check_interval_hours = 24
# URL of the signed update manifest. Override only when mirroring releases.
manifest_url = "https://github.com/quiet-node/thuki/releases/latest/download/latest.json"
```

## Reading the reference tables

Every domain below is shown as a single table that lists **all** constants Thuki uses in that area: both the ones you can tune in `config.toml` and the ones baked in at compile time. The columns are:

- **Constant**: the TOML key (tunable) or Rust/TypeScript identifier (baked-in).
- **Default**: the value Thuki ships with.
- **Tunable?**: `Yes` if editable via `config.toml`, `No` if compiled in.
- **Why not tunable**: only filled for baked-in constants; explains why it is locked.
- **Bounds**: the allowed range for tunable numbers. Values outside this range are reset to the default and a warning is logged.
- **Description**: what the constant controls, in plain language. For tunable numbers, this also explains what raising or lowering the value actually does for you.

## Reference

### `[inference]`

Thuki reaches a model through a **provider**. `active_provider` names which one is used; each provider is described by a `[[inference.providers]]` block. Two kinds exist: **Built-in**, the bundled llama.cpp `llama-server` that Thuki spawns and manages itself (no setup, the default on a fresh install); and **Ollama**, reached over HTTP at a configurable URL, local or remote.

Each provider keeps its own selected `model`. For the built-in engine, models are GGUF files Thuki downloads itself: pick a curated starter (or paste a Hugging Face repo id) in onboarding or Settings → Models → Discover, and manage installed models from the same place. For Ollama, Thuki discovers installed models live from the `/api/tags` endpoint; pull a model with `ollama pull <slug>` and select it. In every case the choice is written to that provider's `model` field, and when no model is installed and none has been chosen, Thuki refuses to dispatch a chat request and surfaces a "Pick a model" prompt.

Upgrading from an older version is automatic: an older config is migrated in place on first load, so your provider and selected model carry over with no manual steps.

| Constant          | Default    | Tunable? | Bounds              | Description                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| :---------------- | :--------- | :------- | :------------------ | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `active_provider` | `"builtin"` | Yes      | id of a provider    | Which provider receives inference. Must match the `id` of one of the `[[inference.providers]]` entries; an empty or dangling value resets to `builtin`.                                                                                                                                                                                              |
| `num_ctx`         | `16384`    | Yes      | `[2048, 1048576]`   | Context window size in tokens sent to the active provider with every request. For the built-in engine, the value becomes `--ctx-size` when the `llama-server` process starts, so changing it restarts the engine. For Ollama, warmup and chat share this value so the same runner instance and its cached KV prefix for the system prompt are reused: they must match or Ollama creates a second runner and the warmup saves nothing. Ollama silently clamps this to the model's physical maximum. Raise to fit longer conversations: the KV cache grows roughly linearly with the context size (the model weights stay the same), so each doubling roughly doubles its memory footprint; benchmark on your hardware before pushing it high, and lower to reclaim memory. See [Tuning the Context Window](./tuning-context-window.md). |
| `keep_warm_inactivity_minutes` | `0` | Yes | `-1` or `[0, 1440]` | Minutes of inactivity before Thuki releases the active model from memory. Governs both local providers: the built-in engine stops its sidecar to free RAM, and Ollama is told to release the model from memory. `0` uses the provider's natural short default (about 5 minutes): Ollama defers to its own timer, the built-in engine applies its own ~5-minute timer (`DEFAULT_BUILTIN_IDLE_MINUTES`). `-1` keeps the model resident forever. Raise for longer sessions between uses; lower to reclaim memory sooner. |

Each `[[inference.providers]]` block has these fields:

| Field      | Description                                                                                                                                                  |
| :--------- | :--------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`       | Stable identifier referenced by `active_provider`. The `builtin` and `ollama` ids are seeded automatically.                                                  |
| `kind`     | `"builtin"` or `"ollama"`. Determines how Thuki talks to the provider.                                       |
| `label`    | Human-readable name shown in Settings.                                                                                                                       |
| `base_url` | For the `ollama` kind: the server's base URL, defaults to `http://127.0.0.1:11434` if empty (then re-seeded). Empty for the `builtin` kind. |
| `model`    | The model selected for this provider, written when you pick one. Empty means "none chosen yet".                                                              |

If the active model has been removed from Ollama between launches, Thuki silently falls back to the first installed model the next time you open the picker. If no models are installed at all, the next request surfaces a "Model not found" error with the exact `ollama pull <name>` command to run.

The table below also lists the baked-in safety limits that govern Thuki's communication with provider HTTP APIs (Ollama and the Hugging Face Hub used for model downloads) and the lifecycle of the built-in engine process. None are tunable.

| Constant                                    | Default  | Tunable? | Why not tunable                                                                                                                                                         | Bounds | Description                                                                                                                                                                          |
| :------------------------------------------ | :------- | :------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :----- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DEFAULT_OLLAMA_TAGS_REQUEST_TIMEOUT_SECS`  | `5 s`    | No       | Protocol cap on a hung daemon to keep the UI responsive. A longer timeout would wedge the model picker; a shorter one would false-trigger on a momentarily slow daemon. | —      | How long Thuki waits for Ollama's `/api/tags` endpoint to respond before giving up. If Ollama accepts the connection but never replies, this prevents the picker from stalling.      |
| `DEFAULT_OLLAMA_SHOW_REQUEST_TIMEOUT_SECS`  | `5 s`    | No       | Protocol cap on a hung daemon to keep the UI responsive. Same rationale as the tags timeout above.                                                                      | —      | How long Thuki waits for Ollama's `/api/show` endpoint to respond before giving up. Used when fetching capability flags (vision, thinking) for each installed model.                |
| `MAX_OLLAMA_TAGS_BODY_BYTES`                | `4 MiB`  | No       | Defense-in-depth bound on attacker-controlled response body. A misbehaving or compromised Ollama could otherwise stream an unbounded payload and exhaust memory.        | —      | The largest `/api/tags` response body Thuki will accept. 4 MiB fits thousands of model entries; anything larger is rejected immediately and the request returns an error.            |
| `MAX_OLLAMA_SHOW_BODY_BYTES`                | `4 MiB`  | No       | Defense-in-depth bound on attacker-controlled response body. Same rationale as `MAX_OLLAMA_TAGS_BODY_BYTES`.                                                            | —      | The largest `/api/show` response body Thuki will accept. Full Modelfiles and parameters can be sizable, but 4 MiB is well above any real model; larger responses are rejected.      |
| `MAX_MODEL_SLUG_LEN`                        | `256 B`  | No       | Defense-in-depth bound on adversarial input. Real Ollama slugs are a handful of characters; capping the length stops malformed values long before any network or DB work. | —      | The longest model slug Thuki will accept from `set_active_model`. Anything longer is rejected immediately by `validate_model_slug`.                                                  |
| `VRAM_POLL_INTERVAL_SECS`                   | `5 s`    | No       | Tuning this trades responsiveness against localhost polling load; 5 s is the sweet spot for loopback calls and matches Ollama's internal TTL resolution granularity. | —      | How often Thuki polls Ollama's `/api/ps` to detect VRAM changes made outside Thuki (for example, running `ollama stop` or a TTL expiry). The Settings panel VRAM indicator reflects these changes within one interval. |
| `ENGINE_HEALTH_DEADLINE_SECS`               | `300 s`  | No       | Engine lifecycle contract: this bounds the worst-case "warming up" wait the UI can show before a start is declared failed, so changing it alters the UX contract rather than tuning a preference. | —      | How long Thuki waits for a freshly spawned built-in engine to pass its `/health` check before giving up and killing the process. Large GGUF models loading from a cold disk can legitimately take minutes, so the deadline is generous. |
| `ENGINE_HEALTH_POLL_INTERVAL_MS`            | `250 ms` | No       | Pure loopback-load tuning: 250 ms detects readiness promptly without hammering the local server while it is busy loading the model.                                  | —      | How often Thuki probes the built-in engine's `/health` endpoint while it starts up. A `503` answer means the model is still loading and the poll continues; `200` means ready.       |
| `ENGINE_IDLE_CHECK_INTERVAL_SECS`           | `30 s`   | No       | Internal timer granularity behind the user-facing `keep_warm_inactivity_minutes` knob; 30 s keeps the unload within a minute-scale setting's precision at negligible cost.    | —      | How often the engine runner checks whether the configured idle window has elapsed and the built-in engine should be stopped to free RAM.                                   |
| `DEFAULT_BUILTIN_IDLE_MINUTES`              | `5 min`  | No       | The fixed translation of the `keep_warm_inactivity_minutes = 0` sentinel for the built-in engine, not a separate preference. The built-in engine has no external daemon to defer to, so `0` ("use the provider's natural short default") resolves to this value. Users who want a different timeout set `keep_warm_inactivity_minutes` directly (`N` minutes, or `-1` for forever). | —      | The idle window the built-in engine applies when `keep_warm_inactivity_minutes` is `0`. After this many minutes of inactivity the sidecar is stopped to free RAM. |
| `ENGINE_HEALTH_PROBE_TIMEOUT_SECS`          | `5 s`    | No       | Internal lifecycle contract between the runner and the engine process. A wedged-but-connected server must not park the poll loop forever; loopback probes are normally instant so 5 s is generous. The poll interval and deadline are the user-facing knobs. | —      | How long a single `/health` GET is allowed to take inside the startup poll loop. If the engine has accepted the TCP connection but stopped responding, this timeout causes the probe to return an error (treated as Wait and retried after `ENGINE_HEALTH_POLL_INTERVAL_MS`). |
| `ENGINE_COMMAND_QUEUE_CAPACITY`             | `64`     | No       | Bounds memory under command bursts; 64 slots is ample for all UI-driven traffic (Ensure, Touch, SetIdleMinutes, Shutdown) under any realistic usage pattern. | —      | Capacity of the bounded `mpsc` channel that carries commands from `EngineHandle` to the runner actor task. Back-pressure from a full queue is not observable in normal use. |
| `ENGINE_STDERR_TAIL_LINES`                  | `20`     | No       | Defense-in-depth bound on captured subprocess output: 20 lines cover the load-error block `llama-server` prints on exit without retaining its whole log. | —      | Number of trailing `llama-server` stderr lines the runner keeps so a crash can report the engine's own reason (e.g. `unknown model architecture`) instead of a generic message. |
| `ENGINE_STDERR_TAIL_LINE_MAX_BYTES`         | `500`    | No       | Defense-in-depth bound on attacker-influenced data: a single pathological newline-less stderr line (e.g. an enormous architecture string echoed from crafted GGUF metadata) is capped during the read, so neither peak read buffering nor the retained tail can grow without limit. | —      | Maximum bytes buffered and retained per captured engine stderr line. |
| `ENGINE_CRASH_FALLBACK_MESSAGE`             | `"engine process exited unexpectedly"` | No | Internal diagnostic fallback surfaced only when the real reason is unavailable; not meaningful to expose. | n/a | Reason reported when the built-in engine process exits without leaving any stderr to capture (e.g. an external `SIGKILL`). |
| `DOWNLOAD_PROGRESS_MIN_INTERVAL_MS`         | `500 ms` | No       | Pure IPC hygiene: a fast local connection can deliver thousands of chunks per second and the UI only needs a few updates per second, so throttling below the UI refresh rate is invisible to the user. | —      | Minimum interval between `Progress` events emitted while a model file downloads. An update is also emitted whenever at least 1% of the file has arrived since the last one, whichever comes first, and a final 100% update always precedes verification. |
| `BLOB_HASH_BUFFER_BYTES`                     | `4 MiB`  | No       | Internal I/O buffer with no user-visible effect beyond verify speed. A few-MB buffer turns hashing a multi-GB blob into a few hundred reads instead of hundreds of thousands. | —      | Read-buffer size for streaming a downloaded blob through SHA-256 during verification. The common path hashes bytes as they download, so this applies only to a full-length partial left from a prior run or a resumed download's on-disk prefix. |
| `DEFAULT_MAX_CONCURRENT_DOWNLOADS`           | `3`      | No       | Defense-in-depth bound against the resource exhaustion of issue #296 (unbounded parallel downloads plus an auto-load froze a memory-constrained Mac). Exposing it would let a user re-introduce the very failure the cap exists to prevent. | —      | The most model downloads allowed to transfer bytes at once. A start beyond the cap waits (surfacing a `Queued` state) for a slot before opening its HTTP transfer, rather than running concurrently. |
| `DEFAULT_DOWNLOAD_DISK_HEADROOM_BYTES`       | `2 GiB`  | No       | Defense-in-depth floor against the disk-fill failure of issue #296, not a preference. Lowering it would re-open that failure by letting a download fill the volume to the brim and wedge the machine. | —      | Free-space headroom kept above a download's own byte needs, enforced both in the pre-download preflight and the periodic mid-transfer re-check. A download is refused (or aborted, keeping its partial for resume) when free space would drop below this floor. |
| `DEFAULT_DOWNLOAD_DISK_RECHECK_INTERVAL_BYTES` | `256 MiB` | No     | Internal safety-probe cadence with no user-visible effect. It only sets how often free space is re-probed mid-transfer; the headroom floor above is what actually governs the abort. | —      | How many bytes a transfer writes between successive free-disk re-checks, so a long download that fills the volume after its preflight passed is caught within a few hundred MB and aborted cleanly. |
| `MAX_HF_API_BODY_BYTES`                     | `4 MiB`  | No       | Defense-in-depth bound on attacker-controlled data from a remote service, mirroring `MAX_OLLAMA_TAGS_BODY_BYTES`. | —      | The largest Hugging Face API response body (repo file listings) Thuki will accept while resolving a model to download. Larger responses are rejected mid-stream and the request returns an error. |
| `MAX_GGUF_KV_COUNT`                         | `4096`   | No       | Defense-in-depth bound on a downloaded GGUF's metadata-key count. A corrupt or hostile `metadata_kv_count` could otherwise drive an unbounded scan; real models carry a few dozen entries, so 4096 never truncates legitimate metadata. | —      | The most GGUF metadata key-value pairs the reasoning classifier scans when reading a downloaded model's chat template. Scanning stops at the cap. |
| `MAX_GGUF_KEY_BYTES`                        | `1 KiB`  | No       | Defense-in-depth bound on a downloaded GGUF's metadata-key length. Keys are short dotted identifiers (`tokenizer.chat_template`); capping the length stops a corrupt length field from forcing a large allocation. | —      | The longest GGUF metadata key the reasoning classifier will read. A longer key stops the scan. |
| `MAX_GGUF_STRING_BYTES`                     | `4 MiB`  | No       | Defense-in-depth bound on a downloaded GGUF's string values. Real chat templates run a few KB to ~100 KB; 4 MiB never truncates one while bounding the memory a corrupt length field can demand. | —      | The largest GGUF string value (the chat template or architecture) the reasoning classifier will materialize. A larger value stops the scan and the model relies on the runtime backstop instead. |
| `HF_API_TIMEOUT_SECS`                       | `15 s`   | No       | Protocol cap on a hung remote service so the download UI cannot stall on metadata resolution; 15 s is generous for a small metadata call over the internet. | —      | How long Thuki waits for a Hugging Face API metadata call (repo file listing) to respond before giving up. Applies to resolving pasted repo ids and listing a repo's GGUF files, not to the model download itself. |
| `HF_BASE_URL`                               | `https://huggingface.co` | No | Single origin for model metadata and downloads. Provenance comes from the pinned repo revisions in the curated starter registry, and those pins are only meaningful against the canonical Hub; an arbitrary mirror could serve different content under the same revision ids. | — | The Hugging Face origin Thuki uses for all model metadata calls and blob downloads. Every starter in the registry pins a repo at an exact revision and carries a compiled-in sha256 digest checked after download; the digest catches truncation, bit rot, and resume corruption, while the pinned revision on the canonical Hub is what fixes which content is fetched. |
| `HF_SEARCH_LIMIT`                           | `30`     | No       | The per-page step for the in-app model browser. The "Load more" control raises the requested page size in multiples of this value, so it is a layout step rather than a user preference. | —      | How many GGUF model repos the first page of an in-app Hugging Face search returns, most-downloaded first. |
| `HF_SEARCH_LIMIT_MAX`                        | `120`    | No       | Defense-in-depth bound on request size: "Load more" grows the requested page size in `HF_SEARCH_LIMIT` steps, and this caps the largest single request so a runaway page count cannot ask the Hub for an unbounded result set. | —      | The largest page size a single in-app Hugging Face search request may ask for, regardless of how many times "Load more" was pressed. |
| `MAX_MODEL_CONTEXT_LENGTH`                   | `1 M`    | No       | Defense-in-depth bound on attacker-controlled GGUF metadata: a repo's `context_length` is editable (`gguf_set_metadata.py`) and occasionally inflated, so a value above this sane ceiling is treated as untrustworthy and dropped rather than shown. Mirrors the `num_ctx` upper bound; 1 M tokens covers every current model. | — | The largest model context window Thuki will trust and display from a Browse-all repo's parsed GGUF metadata. A larger declared value is dropped (no context window shown) rather than rendered. Curated Staff Picks models carry a hand-vetted value in the registry instead. |
| `RUNTIME_OVERHEAD_GB`                        | `2.0`    | No       | Feeds the approximate RAM-fit hint shown in Library and Discover only; the authoritative per-starter memory estimates live in the model registry. A user-tunable overhead would imply a precision the hint does not claim. | —      | Resident-memory overhead added on top of a model's weights size (KV cache plus runtime buffers) when estimating whether it fits in this Mac's RAM. |
| `MAX_HF_SEARCH_QUERY_LEN`                   | `200 bytes` | No    | Defense-in-depth bound on attacker-influenced input: the query reaches the fixed Hub host (no SSRF) and is percent-encoded by the client, but an unbounded string is still rejected to cap request size. | —      | The longest search string Thuki sends to the Hugging Face model search. A longer query is rejected before any network call. |
| `MAX_SSE_LINE_BYTES`                        | `1 MiB`  | No       | Defense-in-depth bound on attacker-controlled stream data. A malicious or broken chat server could otherwise grow a single stream line without limit and exhaust memory. | —      | The longest single Server-Sent-Events line Thuki accepts while streaming a chat response over the built-in engine's `/v1` endpoint. A stream line exceeding this aborts the response with an error. |
| `DEFAULT_STARTUP_SAFE_MODE_THRESHOLD`       | `1`      | No       | Crash-loop safety mechanism, not a preference. After this many consecutive unclean launches (a launch marked dirty at startup that never reached a healthy state), Thuki enters safe mode and skips auto-loading the active model. Threshold `1` trips on a single unclean launch on purpose: the cost of safe mode (the user loads the model manually) is tiny next to the cost of repeating a whole-machine freeze. Letting a user raise it would re-arm exactly that freeze. | —      | How many consecutive unclean launches trip the startup circuit breaker into safe mode. In safe mode the no-user-action model auto-prime is skipped; the model still loads on the user's first message. The reset signal is `mark_startup_healthy`, not a clean exit (Thuki quits only from the tray, so a clean-exit signal almost never fires). |
| `DEFAULT_STARTUP_GUARD_FILENAME`            | `"startup_guard.json"` | No | Internal sentinel filename used by the launch circuit breaker next to `config.toml`; not meaningful to expose and easy to break by typo. | n/a | Filename of the JSON sentinel that records the launch-dirty flag and consecutive-unclean count so the circuit breaker survives app restarts. Lives in the same directory as `config.toml`. |

### `[prompt]`

Controls the personality and instructions Thuki gives to the AI at the start of every conversation.

| Constant                        | Default                                | Tunable? | Why not tunable                                                                                                                                       | Bounds     | Description                                                                                                                                                                                                                                            |
| :------------------------------ | :------------------------------------- | :------- | :---------------------------------------------------------------------------------------------------------------------------------------------------- | :--------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `system`                        | full built-in body (~6 KB)             | Yes      | —                                                                                                                                                     | any string | The full secretary personality prompt, seeded into your `config.toml` on first run. It becomes authoritative only once you save it through Settings (which sets `system_customized = true`); until then it is treated as a cached copy of the built-in default and is refreshed to the current `DEFAULT_SYSTEM_PROMPT_BASE` on every load, so app upgrades that ship a new prompt reach you automatically. To make a hand edit in the TOML stick, also set `system_customized = true`. Saving an empty value via Settings sends no persona at all. The slash-command appendix is always added on top, so `/search` etc. work either way. |
| `DEFAULT_SYSTEM_CUSTOMIZED`    | `false`                                | No       | Internal authority flag. Set to `true` the first time the user saves the system prompt via Settings. While `false`, the persisted `system` is treated as a cached default and is overwritten by `DEFAULT_SYSTEM_PROMPT_BASE` on every load; once `true`, the stored value (including an explicit empty) is respected verbatim. Not user-tunable because exposing it would let users suppress the safety net that keeps non-customizing installs on the current built-in persona across upgrades. | — | Tracks whether the user has ever explicitly saved a system prompt through the Settings UI. |
| `DEFAULT_SYSTEM_PROMPT_BASE`    | `prompts/system_prompt.txt`            | No       | The shipped built-in prompt. Seeds `system` on first run, and is reapplied on every load for any config not customized via Settings (`system_customized = false`), so edits to this file reach all non-customizing installs. | —          | Source-of-truth file used to seed and refresh `system`.                                                                                                                                                                                                                                  |
| `SLASH_COMMAND_PROMPT_APPENDIX` | `prompts/generated/slash_commands.txt` | No       | Auto-generated from the slash-command registry at build time. Editing by hand would desync the AI's understanding of the commands from the real ones. | —          | The list of slash commands (`/search`, `/screen`, etc.) Thuki tells the AI about so it knows what each one does. Always added on top of your `system` prompt.                                                                                          |

### `[window]`

UI configuration for the floating Thuki window: geometry knobs and input attachment limits. The collapsed-bar height and the close-animation deadline are baked into the frontend (see `App.tsx`) because their effective range is invisible to users (collapsed height is overwritten by the ResizeObserver within a frame; the hide delay sits below normal perception across its usable range and creates a visible pop if dropped below the exit-animation duration).

| Constant          | Default | Tunable? | Why not tunable | Bounds            | Description                                                                                                                                                                            |
| :---------------- | :------ | :------- | :-------------- | :---------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `overlay_width`   | `600.0` | Yes      | —               | `[200.0, 2000.0]` | How wide the floating Thuki window is, in pixels. Raise for wider input/chat at the cost of more screen space; lower to keep Thuki compact.                                            |
| `max_chat_height` | `648.0` | Yes      | —               | `[200.0, 2000.0]` | The largest the chat window can grow to as conversation gets longer. Raise to see more chat history without scrolling; lower to keep Thuki from taking over your screen on long chats. |
| `max_images`      | `3`     | Yes      | —               | `[1, 20]`         | Maximum number of images you can manually attach to a single message by pasting or dragging. A /screen capture always counts as one extra on top of this limit. Raise for richer visual context per message; lower to keep prompts compact. |
| `text_base_px`    | `15.0`  | Yes      | —               | `[11.0, 22.0]`    | Base font size for chat text and the AskBar input, in CSS pixels. Drives the `--thuki-text-base` CSS variable consumed by the AI markdown body, the user chat bubble text, and the AskBar textarea (plus its caret-tracking mirror). Other surfaces (Settings panel, onboarding) keep fixed sizes. Raise for easier-to-read conversation text; lower to fit more text on screen. |
| `text_line_height` | `1.5` | Yes      | —               | `[1.0, 2.5]`      | Line-height multiplier applied to chat text and the AskBar input. Drives the `--thuki-text-line-height` CSS variable. Raise for airier, easier-to-skim replies; lower to fit more lines on screen. |
| `text_letter_spacing_px` | `0.0` | Yes | —             | `[-0.5, 2.0]`     | Extra space between characters, in CSS pixels. Drives the `--thuki-text-letter-spacing` CSS variable. Raise for airier letters; drop below zero to tighten the typography. |
| `text_font_weight` | `500` | Yes      | —               | `{400, 500, 600, 700}` | CSS `font-weight` applied to chat and AskBar text. Drives the `--thuki-text-font-weight` CSS variable. Only the four loaded Nunito weights are accepted; off-grid values reset to the default. Raise for a heavier presence; lower for a lighter look. |
| `COLLAPSED_WINDOW_HEIGHT` | `80 px` | No | Frontend constant; overwritten by ResizeObserver before the frame renders, so any value in the user-visible range produces identical results. | — | The initial height of the collapsed input bar, in pixels. Overwritten by ResizeObserver on every render, so the value the user sees is always determined dynamically. |
| `HIDE_COMMIT_DELAY_MS` | `350 ms` | No | Frontend constant; the value sits below normal perception across its usable range and creates a visible pop if dropped below the exit-animation duration. | — | How long Thuki waits after you close the window before it hides the underlying NSPanel. Keeps the exit animation from being cut off. |

### `[quote]`

Controls how text you select in another app (and bring to Thuki) appears as a quote in the input bar, and how much of it actually gets sent to the AI.

| Constant             | Default | Tunable? | Why not tunable | Bounds       | Description                                                                                                                                                                                                                                                        |
| :------------------- | :------ | :------- | :-------------- | :----------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `max_display_lines`  | `4`     | Yes      | —               | `[1, 100]`   | How many lines of the quoted text are shown as a preview in the input bar. The full text is still sent to the AI; this only affects what you see. Raise to preview more of the quote at a glance; lower to keep the input bar compact.                             |
| `max_display_chars`  | `300`   | Yes      | —               | `[1, 10000]` | How many characters of the quoted text are shown as a preview in the input bar. Same idea as `max_display_lines`: the full text is still sent to the AI. Raise for a longer preview; lower to keep the bar compact.                                                |
| `max_context_length` | `4096`  | Yes      | —               | `[1, 65536]` | How many characters of the quoted text are actually sent to the AI. Anything past this is cut off. Raise if you quote long passages and want the AI to see all of it; lower if your model has a small context window or you want to save tokens on big selections. |

### `[behavior]`

Controls what happens to a `/rewrite` or `/refine` result: whether Thuki writes it straight back into the app you were using, or waits for you to send it back yourself.

| Constant       | Default | Tunable? | Why not tunable | Bounds | Description                                                                                                                                                                                                                                                                                                          |
| :------------- | :------ | :------- | :-------------- | :----- | :--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `auto_replace` | `false` | Yes      | —               | —      | When on, a `/rewrite` or `/refine` result is written straight back into the source app, replacing your highlighted text, the moment the rewrite is ready, with no extra click. When off, the rewrite appears in Thuki and you press the Replace button to send it back. The Replace button is available either way. |
| `auto_close`   | `false` | Yes      | —               | —      | When on, the Thuki overlay closes itself right after a `/rewrite` or `/refine` result is replaced back into the source app, whether the replace happened automatically (`auto_replace`) or from a manual Replace click. Only closes on a successful replace. Independent of `auto_replace`. Turn on for a one-shot rewrite-and-dismiss flow; leave off to keep Thuki open and replace repeatedly. |

### `[search]`

Settings for the `/search` command, which lets the AI search the web and read pages to answer your question. Covers where Thuki's local search and page-reader services live, how hard it should try to find good results, and how long to wait at each step.

URLs must include scheme, host, and port, with no path. Thuki appends the rest (`/search`, `/extract`) automatically. If you leave a URL empty in your config, Thuki uses the default; if you put a number outside its allowed range, Thuki resets it to the default and logs a warning.

For security, both URLs default to your local machine (`127.0.0.1`) and should stay there. Pointing them at a remote server breaks Thuki's sandbox isolation: the page reader would fetch arbitrary URLs on behalf of the AI from a host that may have access to private networks.

| Constant                        | Default                    | Tunable? | Why not tunable                                                                                                                              | Bounds        | Description                                                                                                                                                                                                                                                                                                                           |
| :------------------------------ | :------------------------- | :------- | :------------------------------------------------------------------------------------------------------------------------------------------- | :------------ | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `searxng_url`                   | `"http://127.0.0.1:25017"` | Yes      | —                                                                                                                                            | non-empty URL | Where Thuki's local search engine (SearXNG) is running. SearXNG is the service that actually sends your query to Google, Bing, etc. and brings the results back. Change only if you moved it to a different port or host.                                                                                                             |
| `reader_url`                    | `"http://127.0.0.1:25018"` | Yes      | —                                                                                                                                            | non-empty URL | Where Thuki's local web-page reader is running. The reader is the service that opens promising URLs, strips out ads/menus/scripts, and hands the clean text back so the AI can read it. Change only if you moved the service.                                                                                                         |
| `searxng_max_results`           | `10`                       | Yes      | —                                                                                                                                            | `[1, 20]`     | How many results SearXNG returns for each query, before Thuki ranks them and picks the best ones to read. Raise for wider coverage (more candidate URLs to pick from); lower for faster, narrower searches.                                                                                                                           |
| `max_iterations`                | `3`                        | Yes      | —                                                                                                                                            | `[1, 10]`     | How many rounds of searching the AI is allowed to do for a single question. If the first round of results does not have enough info, the AI generates new queries and tries again. Raise for hard, multi-step questions that need more digging; lower if you want answers faster and to use fewer tokens (good when results give up). |
| `top_k_urls`                    | `10`                       | Yes      | —                                                                                                                                            | `[1, 20]`     | How many web pages Thuki actually opens and reads after picking the most promising ones from the search `searxng_max_results`. Raise to give the AI more sources to pull facts from in its answer; lower for faster searches with less to read (and slightly less coverage).                                                          |
| `search_timeout_s`              | `20`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | How long (in seconds) Thuki waits for SearXNG to come back with search results before giving up on a single query. Raise this if you have a slow internet connection. Lowering it only causes searches to give up before they would have succeeded.                                                                                   |
| `reader_per_url_timeout_s`      | `10`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | How long (in seconds) Thuki waits for one single web page to load before giving up on it and moving on. Raise this for slow websites that take a while to respond. Lowering it just makes more pages get skipped.                                                                                                                     |
| `reader_batch_timeout_s`        | `30`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | How long (in seconds) Thuki waits for the whole batch of pages it's reading in parallel to finish. Must be larger than `reader_per_url_timeout_s`; if it's not, Thuki automatically bumps it to `reader_per_url_timeout_s + 5`. Raise on slow connections so a few slow pages don't kill the whole batch.                             |
| `judge_timeout_s`               | `30`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | How long (in seconds) Thuki waits for the AI to decide whether the search results are good enough to answer your question. Raise this if your local AI model is slow on your hardware. Lowering it only causes the judging step to give up early.                                                                                     |
| `router_timeout_s`              | `45`                       | Yes      | —                                                                                                                                            | `[1, 300]`    | How long (in seconds) Thuki waits for the AI to decide whether your question even needs a web search and to plan the first queries. Raise this if your local AI model is slow on your hardware. Lowering it only causes the planning step to give up early.                                                                           |
| `GAP_QUERIES_PER_ROUND`         | `3`                        | No       | Drives the judge-normalization cap and the prompt structure; changing it silently alters output quality rather than producing a clear error. | —             | When the AI decides the current results are not enough, it generates this many follow-up search queries to try. Three is the right balance between coverage and noise for the prompts Thuki uses.                                                                                                                                     |
| `CHUNK_TOKEN_SIZE`              | `500`                      | No       | Downstream synthesis prompts assume this exact chunk size; rerank scoring is calibrated to it.                                               | —             | Long web pages are split into smaller pieces ("chunks") so the AI can pick the most relevant parts. This is roughly how many tokens go into each chunk.                                                                                                                                                                               |
| `TOP_K_CHUNKS`                  | `8`                        | No       | Coupled to the synthesis prompt's context budget; larger values overflow the model window.                                                   | —             | After splitting pages into chunks and scoring them, this many of the highest-scoring chunks are sent to the AI to write the final answer.                                                                                                                                                                                             |
| `DEFAULT_READER_RETRY_DELAY_MS` | `500`                      | No       | Balances pressure on the sandbox reader against perceived responsiveness; no user signal that it needs to vary.                              | —             | If a page fetch fails, this is how long (in milliseconds) Thuki waits before trying again, so the reader service does not get hammered with retries.                                                                                                                                                                                  |
| `DEFAULT_MAX_QUERY_CHARS`       | `500`                      | No       | Defense-in-depth bound on outgoing queries to external engines; exposing it lets a malformed prompt DOS upstream services.                   | —             | The longest a search query can be (in characters) before Thuki trims it. A safety cap on what gets sent to the search engine; the AI's queries are normally well under this.                                                                                                                                                          |
| `DEFAULT_MAX_SNIPPET_CHARS`     | `500`                      | No       | Defense-in-depth bound on incoming text from external engines; exposing it lets a malicious result flood the rerank prompt.                  | —             | The longest each search-result snippet (the title and short blurb under each link) can be before Thuki trims it. A safety cap to keep an oversized result from blowing up the AI's prompt.                                                                                                                                            |

### `[debug]`

Records every chat conversation and `/search` session as JSON-Lines under `app_data_dir/traces/{chat,search}/<conversation_id>.jsonl`. Off by default; toggleable from Settings. Trace files stay on your disk and are never uploaded.

| Field           | Default | Tunable? | Why not tunable | Bounds | Description                                                                  |
| :-------------- | :------ | :------- | :-------------- | :----- | :--------------------------------------------------------------------------- |
| `trace_enabled` | `false` | Yes      | —               | —      | Records every chat conversation and `/search` session to disk for debugging. |

### `[updater]`

Controls how Thuki polls for new releases. The actual download, signature verification, and binary swap are handled by the bundled Tauri updater plugin against a signed manifest hosted on GitHub Releases. The manifest is verified against an ed25519 public key compiled into the app, so a hijacked release cannot push a malicious binary to existing installs.

| Field                  | Default                                                                              | Tunable? | Why not tunable | Bounds   | Description                                                                                                                                                                                  |
| :--------------------- | :----------------------------------------------------------------------------------- | :------- | :-------------- | :------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `auto_check`           | `true`                                                                               | Yes      | n/a             | n/a      | Whether Thuki polls for updates automatically. When false, only the "Check now" button in Settings triggers a check. The tray badge and Settings banner still appear if a check finds an update. |
| `check_interval_hours` | `24`                                                                                 | Yes      | n/a             | `1..168` | Hours between automatic background checks. Raise to spend less bandwidth on update polling; lower to surface new releases sooner. The interval also gates the startup check after a freshly resumed session. |
| `manifest_url`         | GitHub releases default                                                              | Yes      | n/a             | n/a      | URL of the signed update manifest. Override only when mirroring releases (for example, an internal release feed). Empty values fall back to the default URL.                                |
| `MAX_UPDATER_SNOOZE_HOURS` | `8760`                                                                           | No       | Defense-in-depth bound on `hours` arriving from the frontend IPC; prevents `u64` arithmetic in the snooze handlers from wrapping if a hostile or buggy caller supplies an extreme value. | n/a      | Maximum number of hours a "snooze update" request can defer the next nag. Caps at one year so the deadline math cannot overflow even in the worst case.                                     |
| `DEFAULT_UPDATER_STATE_FILENAME` | `"updater_state.json"`                                                     | No       | Internal sidecar filename used for snooze persistence next to `config.toml`; not meaningful to expose and easy to break by typo. | n/a      | Filename of the JSON sidecar that records snooze deadlines so they survive app restarts. Lives in the same directory as `config.toml`.                                                      |

### `[activation]` (not in TOML)

Settings for the double-tap-Control hotkey that opens Thuki, plus the macOS Accessibility permission check. None of these are user-tunable: the hotkey listener runs in a low-level system thread that cannot read live config, so changing them would require restructuring the keyboard plumbing.

| Constant                   | Default  | Tunable? | Why not tunable                                                                                                                           | Bounds | Description                                                                                                                                                 |
| :------------------------- | :------- | :------- | :---------------------------------------------------------------------------------------------------------------------------------------- | :----- | :---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ACTIVATION_WINDOW`        | `400 ms` | No       | Event-tap callback cannot read Tauri managed state; would require a redesign to expose. No user has reported needing a different cadence. | —      | How fast you have to double-tap Control to open Thuki: the second tap must happen within this many milliseconds of the first, otherwise it does not count.  |
| `ACTIVATION_COOLDOWN`      | `600 ms` | No       | Same as above.                                                                                                                            | —      | After Thuki opens or closes, this is how long it ignores another double-tap. Prevents accidental rapid-fire toggling when you tap too many times in a row.  |
| `KC_PRIMARY_L`             | `0x3b`   | No       | macOS hardware key code for left Control. Not user-meaningful; wrong value would brick activation.                                        | —      | The internal macOS hardware code for the LEFT Control key. This is not something you set; it is just the number macOS uses to identify that key.            |
| `KC_PRIMARY_R`             | `0x3e`   | No       | macOS hardware key code for right Control. Not user-meaningful; wrong value would brick activation.                                       | —      | The internal macOS hardware code for the RIGHT Control key. Same idea as `KC_PRIMARY_L`.                                                                    |
| `MAX_PERMISSION_ATTEMPTS`  | `6`      | No       | Internal retry budget for the Accessibility prompt; no user-facing reason to tune.                                                        | —      | When you first run Thuki, it asks for Accessibility permission so it can listen for the Control key. This is how many times Thuki re-checks while it waits. |
| `PERMISSION_POLL_INTERVAL` | `5 s`    | No       | Same as above.                                                                                                                            | —      | How often (in seconds) Thuki re-checks for Accessibility permission while it waits for you to grant it.                                                     |

### `[vision]` (not in TOML)

Limits and quality settings for images you attach to a message (whether you drag them in or capture them with `/screen`). The number of images per message is the tunable `[window] max_images` (default 3) plus one slot reserved for a `/screen` capture; the size and quality settings below are fixed, tuned for the best balance of file size and AI accuracy.

| Constant                 | Default   | Tunable? | Why not tunable                                                                                                          | Bounds | Description                                                                                                                                                                          |
| :----------------------- | :-------- | :------- | :----------------------------------------------------------------------------------------------------------------------- | :----- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MAX_IMAGE_SIZE_BYTES`   | `30 MiB`  | No       | Frontend rejection threshold aligned with local vision models' practical decode ceiling.                                             | —      | The biggest image file you can attach (30 MB). Files larger than this are rejected before they're even processed, so an oversized image cannot crash the AI.                         |
| `MAX_DIMENSION`          | `1920 px` | No       | Downscale target that balances vision-model accuracy against payload size.                                               | —      | If an image is wider or taller than this many pixels, Thuki shrinks it to fit (keeping its aspect ratio). Keeps file sizes manageable without hurting how well vision models see it. |
| `JPEG_QUALITY`           | `85`      | No       | Balances file size against visual fidelity for vision models; changes would invalidate historical saved images.          | —      | The compression level Thuki uses when saving attached images as JPEG (on a 1–100 scale; higher = better quality and bigger file). 85 is the sweet spot for vision models.            |

### `[history]` (not in TOML)

Settings for the conversation history panel (where you scroll back through past chats and search them). Not user-tunable.

| Constant             | Default | Tunable? | Why not tunable                                                                                                                              | Bounds | Description                                                                                                                                                                                                                                        |
| :------------------- | :------ | :------- | :------------------------------------------------------------------------------------------------------------------------------------------- | :----- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SEARCH_DEBOUNCE_MS` | `200`   | No       | UX tuning; no meaningful user signal for changing this. Raising it makes search feel sluggish; lowering it wastes cycles on every keystroke. | —      | When you type in the history search box, Thuki waits this many milliseconds after your last keystroke before actually running the search. Stops Thuki from running a fresh search on every single character you type, while still feeling instant. |
| `STRIP_PATTERNS`     | 17 token strings | No | Defense-in-depth bound on external/attacker-controlled data: special turn-boundary tokens leaked by fine-tuned models would corrupt cross-model history if persisted. Exposing this list as a config knob would let a malformed or adversarial model response disable the sanitization layer. | — | The set of special delimiters (e.g. `<\|im_start\|>`, `[INST]`, `<think>`) that major model families use internally. Some fine-tuned models leak these into `message.content`; Thuki strips them before storing an assistant reply and again at render time so switching between model families does not produce visible garbage in the chat window. The TypeScript mirror of this list (`src/utils/sanitizeAssistantContent.ts`) must be kept in sync when new model families are added. |

### Email capture (not in TOML)

Backs the optional "Help shape Thuki" email ask (the onboarding roadmap screen and Settings ▸ About). Thuki sends your email only when you click the button, and POSTs just `{ email, source }` to a public proxy that holds the email-service key, so no secret ever lives in the app. Not user-tunable.

| Constant                     | Default                          | Tunable? | Why not tunable                                                                                                                                                  | Bounds | Description                                                                                                                                                                            |
| :--------------------------- | :------------------------------- | :------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------- | :----- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DEFAULT_SUBSCRIBE_ENDPOINT` | `https://thuki.app/api/subscribe` | No       | Fixed external-service endpoint, not a knob: the proxy at this origin holds the email-service key and enforces the contract, so pointing it elsewhere would silently break the subscribe flow. | —      | The public proxy URL the "Help shape Thuki" ask POSTs `{ email, source }` to. A `200` (including already-subscribed) is success; any other response surfaces a generic, retryable error. |
| `DEFAULT_SUBSCRIBE_TIMEOUT_SECS` | `15` | No       | Internal robustness bound on a one-shot network call, not a preference: it caps how long the sending state can last if the proxy stalls. | —      | Per-request timeout (seconds) for the subscribe POST. If the proxy does not respond in time the request fails into the same generic, retryable error rather than hanging. |

## What happens on bad input

Thuki tries to keep itself running with a working configuration rather than crash on a typo. Here is what it does in each case:

- **The file is missing**: Thuki writes a fresh defaults file and launches normally.
- **A field is missing**: Thuki uses the default for that field; your other settings stay as-is.
- **A field is empty or just whitespace**: Thuki uses the default for that field.
- **A number is outside its allowed range**: Thuki resets that field to the default and logs a warning. (You can see warnings in `Console.app`.)
- **The file is not valid TOML at all**: Thuki renames the broken file to `config.toml.corrupt-<unix_timestamp>` and writes a fresh defaults file. Your old file is kept so you can open it and copy out anything you want to recover.
