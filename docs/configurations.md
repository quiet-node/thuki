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
# Where Thuki finds your local Ollama server. The active model itself is
# selected from the in-app picker (which lists whatever is installed in
# Ollama via /api/tags) and is stored in Thuki's local database, not here.
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
searxng_max_results = 10
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
- **Bounds**: the allowed range for tunable numbers. Values outside this range are reset to the default and a warning is logged.
- **Description**: what the constant controls, in plain language. For tunable numbers, this also explains what raising or lowering the value actually does for you.

## Reference

### `[model]`

Where to find your local Ollama server. The active model itself is **not** a TOML setting: Thuki discovers installed models live from Ollama's `/api/tags` endpoint, lets you pick one from the in-app model picker, and stores that selection in its local SQLite database (`app_config` table). Storing the active slug in TOML would duplicate ground truth from Ollama and break the moment you remove a model with `ollama rm`, so it lives next to the conversation history instead.

| Constant     | Default                    | Tunable? | Why not tunable | Bounds        | Description                                                                                                                                                                                                          |
| :----------- | :------------------------- | :------- | :-------------- | :------------ | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ollama_url` | `"http://127.0.0.1:11434"` | Yes      | —               | non-empty URL | The web address where Thuki finds your local Ollama server. The default works if you run Ollama on this machine with its standard port. Change this only if you moved Ollama to a different port or another machine. |

If the active model has been removed from Ollama between launches, Thuki silently falls back to the first installed model the next time you open the picker. If no models are installed at all, the next request surfaces a "Model not found" error with the exact `ollama pull <name>` command to run.

### `[prompt]`

Controls the personality and instructions Thuki gives to the AI at the start of every conversation.

| Constant                        | Default                                | Tunable? | Why not tunable                                                                                                                                       | Bounds     | Description                                                                                                                                                                                                                                            |
| :------------------------------ | :------------------------------------- | :------- | :---------------------------------------------------------------------------------------------------------------------------------------------------- | :--------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `system`                        | `""`                                   | Yes      | —                                                                                                                                                     | any string | Your custom personality or instructions for the AI (for example, "You are a terse Rust expert"). Leave this empty to use Thuki's built-in secretary personality. The list of slash commands is always added on top, so `/search` etc. work either way. |
| `DEFAULT_SYSTEM_PROMPT_BASE`    | `prompts/system_prompt.txt`            | No       | This is the fallback used when `system` is empty. To customize, set `system` instead, edit it in your `config.toml` rather than this file.            | —          | The built-in secretary personality Thuki uses when you have not set a custom `system` prompt.                                                                                                                                                          |
| `SLASH_COMMAND_PROMPT_APPENDIX` | `prompts/generated/slash_commands.txt` | No       | Auto-generated from the slash-command registry at build time. Editing by hand would desync the AI's understanding of the commands from the real ones. | —          | The list of slash commands (`/search`, `/screen`, etc.) Thuki tells the AI about so it knows what each one does. Always added on top of your `system` prompt.                                                                                          |

### `[window]`

Size and animation timing for the floating Thuki window.

| Constant               | Default | Tunable? | Why not tunable | Bounds            | Description                                                                                                                                                                                           |
| :--------------------- | :------ | :------- | :-------------- | :---------------- | :---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `overlay_width`        | `600.0` | Yes      | —               | `[200.0, 2000.0]` | How wide the floating Thuki window is, in pixels. Raise for wider input/chat at the cost of more screen space; lower to keep Thuki compact.                                                           |
| `collapsed_height`     | `80.0`  | Yes      | —               | `[40.0, 400.0]`   | How tall the input bar is before you have asked anything (the small spotlight-style bar). Raise if you frequently paste long prompts and want more visible text; lower for a slimmer initial bar.     |
| `max_chat_height`      | `648.0` | Yes      | —               | `[200.0, 2000.0]` | The largest the chat window can grow to as conversation gets longer. Raise to see more chat history without scrolling; lower to keep Thuki from taking over your screen on long chats.                |
| `hide_commit_delay_ms` | `350`   | Yes      | —               | `[0, 5000]`       | How long (in milliseconds) Thuki keeps the close animation playing before the window actually disappears. Raise for a smoother, more leisurely exit; lower (or set 0) to hide instantly when closing. |

### `[quote]`

Controls how text you select in another app (and bring to Thuki) appears as a quote in the input bar, and how much of it actually gets sent to the AI.

| Constant             | Default | Tunable? | Why not tunable | Bounds       | Description                                                                                                                                                                                                                                                        |
| :------------------- | :------ | :------- | :-------------- | :----------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `max_display_lines`  | `4`     | Yes      | —               | `[1, 100]`   | How many lines of the quoted text are shown as a preview in the input bar. The full text is still sent to the AI; this only affects what you see. Raise to preview more of the quote at a glance; lower to keep the input bar compact.                             |
| `max_display_chars`  | `300`   | Yes      | —               | `[1, 10000]` | How many characters of the quoted text are shown as a preview in the input bar. Same idea as `max_display_lines`: the full text is still sent to the AI. Raise for a longer preview; lower to keep the bar compact.                                                |
| `max_context_length` | `4096`  | Yes      | —               | `[1, 65536]` | How many characters of the quoted text are actually sent to the AI. Anything past this is cut off. Raise if you quote long passages and want the AI to see all of it; lower if your model has a small context window or you want to save tokens on big selections. |

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

Limits and quality settings for images you attach to a message (whether you drag them in or capture them with `/screen`). None of these are user-tunable: the image count is capped by what Ollama's vision models accept, and the size/quality settings are tuned for the best balance of file size and AI accuracy.

| Constant                 | Default   | Tunable? | Why not tunable                                                                                                          | Bounds | Description                                                                                                                                                                          |
| :----------------------- | :-------- | :------- | :----------------------------------------------------------------------------------------------------------------------- | :----- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MAX_IMAGES_PER_MESSAGE` | `4`       | No       | Protocol cap: 3 manual attachments + 1 `/screen` capture. Larger values make requests fail further downstream in Ollama. | —      | The maximum number of images you can attach to a single message: 3 you add yourself, plus 1 captured with `/screen`. Adding more than this just makes the request fail in Ollama.    |
| `MAX_IMAGE_SIZE_BYTES`   | `30 MiB`  | No       | Frontend rejection threshold aligned with Ollama's practical decode ceiling.                                             | —      | The biggest image file you can attach (30 MB). Files larger than this are rejected before they're even processed, so an oversized image cannot crash the AI.                         |
| `MAX_DIMENSION`          | `1920 px` | No       | Downscale target that balances vision-model accuracy against payload size.                                               | —      | If an image is wider or taller than this many pixels, Thuki shrinks it to fit (keeping its aspect ratio). Keeps file sizes manageable without hurting how well vision models see it. |
| `JPEG_QUALITY`           | `85`      | No       | Balances file size against visual fidelity for vision models; changes would invalidate historical saved images.          | —      | The compression level Thuki uses when saving attached images as JPEG (on a 1–100 scale; higher = better quality and bigger file). 85 is the sweet spot for vision models.            |

### `[history]` (not in TOML)

Settings for the conversation history panel (where you scroll back through past chats and search them). Not user-tunable.

| Constant             | Default | Tunable? | Why not tunable                                                                                                                              | Bounds | Description                                                                                                                                                                                                                                        |
| :------------------- | :------ | :------- | :------------------------------------------------------------------------------------------------------------------------------------------- | :----- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SEARCH_DEBOUNCE_MS` | `200`   | No       | UX tuning; no meaningful user signal for changing this. Raising it makes search feel sluggish; lowering it wastes cycles on every keystroke. | —      | When you type in the history search box, Thuki waits this many milliseconds after your last keystroke before actually running the search. Stops Thuki from running a fresh search on every single character you type, while still feeling instant. |

## What happens on bad input

Thuki tries to keep itself running with a working configuration rather than crash on a typo. Here is what it does in each case:

- **The file is missing**: Thuki writes a fresh defaults file and launches normally.
- **A field is missing**: Thuki uses the default for that field; your other settings stay as-is.
- **A field is empty or just whitespace**: Thuki uses the default for that field.
- **A number is outside its allowed range**: Thuki resets that field to the default and logs a warning. (You can see warnings in `Console.app`.)
- **The file is not valid TOML at all**: Thuki renames the broken file to `config.toml.corrupt-<unix_timestamp>` and writes a fresh defaults file. Your old file is kept so you can open it and copy out anything you want to recover.
