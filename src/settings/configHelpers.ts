/**
 * Tooltip copy for every user-tunable Settings field.
 *
 * The strings here MIRROR the per-field rows in `docs/configurations.md`
 * so the in-app `?` tooltip and the long-form documentation tell the
 * same story. When you add or change a tunable, update both this file
 * and the matching table row in the docs in the same commit.
 *
 * Indexed by a `(section, key)` pair. Most keys are the canonical TOML field
 * names from the backend `set_config_field` allowlist; a few (e.g.
 * `inference.ollama_base_url`, `inference.keep_warm`) are display-only keys
 * for values written through dedicated commands such as `set_ollama_url`.
 */

const HELPERS = {
  inference: {
    ollama_base_url:
      'The address where Thuki reaches your Ollama server. The default works if you run Ollama on this Mac with its standard port. Point it at another machine to use Ollama running elsewhere (one server at a time).',
    keep_warm:
      'How long Thuki keeps the active model resident in memory between messages, so the next one skips the cold-load wait. Applies to both local providers (the built-in engine and Ollama); it does not apply to a remote OpenAI-compatible server, whose memory Thuki does not manage. Set "Release after" to −1 to keep it resident indefinitely, 0 to use the provider\'s natural short default (about 5 minutes), or a timeout in minutes so memory is reclaimed when you stop using Thuki for a while.',
    builtin_model:
      'The downloaded model Thuki\'s built-in engine runs. Pick from the models you have downloaded, or use "Download a model" below to grab a curated starter or any GGUF file from a Hugging Face repo.',
    openai_base_url:
      'The address of your OpenAI-compatible server (LM Studio, Jan, llama-server, and similar all expose one). Thuki calls its /v1 endpoints for chat and model listing. Must start with http:// or https://.',
    openai_api_key:
      "The API key sent as a Bearer token to your OpenAI-compatible server, stored only in the macOS Keychain. It is never written to config.toml and never shown again after saving; leave it empty for local servers that don't require one.",
    openai_vision:
      'Whether the selected model accepts image inputs. OpenAI-compatible servers expose no capability probe, so you declare it yourself. Turn it on only if the model truly supports images; otherwise requests with attachments will fail.',
    num_ctx:
      'How much conversation the model keeps in working memory, in tokens. Larger fits longer chats, but the KV cache uses more memory as it grows, so benchmark before pushing it high. For the built-in engine, changing this restarts the engine. Range: 2048–1048576.',
  },
  prompt: {
    system:
      'Your custom personality or instructions for the AI (for example, "You are a terse Rust expert"). Leave this empty to use Thuki\'s built-in secretary personality. The list of slash commands is always added on top, so /search and friends keep working either way.',
  },
  window: {
    overlay_width:
      'How wide the floating Thuki window is, in pixels. Raise for wider input/chat at the cost of more screen space; lower to keep Thuki compact.',
    max_chat_height:
      'The largest the chat window can grow to as conversation gets longer. Raise to see more chat history without scrolling; lower to keep Thuki from taking over your screen on long chats.',
    max_images:
      'How many images you can attach to a single message by pasting or dragging. A /screen capture always adds one more on top of this limit. Raise for richer visual context per message; lower to keep prompts compact.',
    text_base_px:
      'How big chat text and the input bar text appear, in pixels. Affects the AI replies, your own chat messages, and what you type in the input bar. Other parts of Thuki (Settings, onboarding) keep their fixed sizes. Raise for easier-to-read text; lower to fit more on screen.',
    text_line_height:
      'How much vertical space each line of chat text and input text takes, as a multiplier of the font size. Raise for airier, easier-to-skim replies; lower to fit more lines on screen.',
    text_letter_spacing_px:
      'Extra space between characters, in pixels. Affects chat text and the input bar. Raise for airier letters; lower (negative values allowed) to tighten the typography.',
    text_font_weight:
      'How bold the chat and input text appears. Regular is the lightest; Bold is the heaviest. Only the four loaded Nunito weights are available because anything in between would silently fall back to the nearest loaded weight.',
  },
  quote: {
    max_display_lines:
      'How many lines of the quoted text are shown as a preview in the input bar. The full text is still sent to the AI; this only affects what you see. Raise to preview more of the quote at a glance; lower to keep the input bar compact.',
    max_display_chars:
      'How many characters of the quoted text are shown as a preview in the input bar. Same idea as max display lines: the full text is still sent to the AI. Raise for a longer preview; lower to keep the bar compact.',
    max_context_length:
      'How many characters of the quoted text are actually sent to the AI. Anything past this is cut off. Raise if you quote long passages and want the AI to see all of it; lower if your model has a small context window or you want to save tokens on big selections.',
  },
  behavior: {
    auto_replace:
      'When on, a /rewrite or /refine result is written straight back into your app, replacing your highlighted text, with no click. When off, click the Replace button to send it back. Off by default.',
    auto_close:
      'When on, Thuki closes itself after a /rewrite or /refine result is replaced into your app (via Auto-replace or the Replace button). Only if the replace succeeds. Off by default.',
    auto_search:
      'On (default): search the web when a plain message needs live facts. Off: stay local unless you type /search.',
    search_notice_acknowledged:
      'Internal flag set after you dismiss the first-use web-search notice. You do not need to edit this; use Auto search to control searching.',
    auto_save_conversations:
      'On (default): each completed turn is saved to local history without a bookmark click. Off: only an explicit Save keeps the chat.',
    history_retention_days:
      'How many days saved chats are kept by last activity before Thuki prunes them at startup or when you confirm a shorter window. Raise to keep history longer; lower to reclaim disk sooner; set -1 to keep them forever. 0 and other out-of-range values reset to the default of -1.',
    auto_save_notice_acknowledged:
      'Internal flag set after you dismiss the one-shot auto-save chat notice. You do not need to edit this; use Auto-save chats to control saving.',
  },
  debug: {
    trace_enabled:
      'When on, Thuki saves a JSONL trace of every chat and search session to ~/Library/Application Support/com.quietnode.thuki/traces/. Useful for debugging and refining your prompts. Off by default.',
    trace_retention_days:
      'How many days recorded traces are kept on disk before Thuki prunes them at startup or when you change this. Raise to keep history longer; lower to reclaim disk sooner; set -1 to keep them forever. 0 and other out-of-range values reset to the default of 7.',
  },
} as const;

/**
 * Returns the tooltip copy for a `(section, key)` field. Throws in
 * development so a missing entry is caught at the first render rather
 * than shipping a silently-empty tooltip; in production we fall back to
 * an empty string so the row still renders.
 */
export function configHelp<
  S extends keyof typeof HELPERS,
  K extends keyof (typeof HELPERS)[S],
>(section: S, key: K): string {
  return HELPERS[section][key] as string;
}
