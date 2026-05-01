/**
 * Tooltip copy for every user-tunable Settings field.
 *
 * The strings here MIRROR the per-field rows in `docs/configurations.md`
 * so the in-app `?` tooltip and the long-form documentation tell the
 * same story. When you add or change a tunable, update both this file
 * and the matching table row in the docs in the same commit.
 *
 * Indexed by the same `(section, key)` pair the backend's
 * `set_config_field` allowlist uses, so the keys here are guaranteed to
 * be the canonical TOML field names.
 */

const HELPERS = {
  inference: {
    ollama_url:
      'The web address where Thuki finds your local Ollama server. The default works if you run Ollama on this machine with its standard port. Change this only if you moved Ollama to a different port or another machine.',
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
  },
  quote: {
    max_display_lines:
      'How many lines of the quoted text are shown as a preview in the input bar. The full text is still sent to the AI; this only affects what you see. Raise to preview more of the quote at a glance; lower to keep the input bar compact.',
    max_display_chars:
      'How many characters of the quoted text are shown as a preview in the input bar. Same idea as max display lines: the full text is still sent to the AI. Raise for a longer preview; lower to keep the bar compact.',
    max_context_length:
      'How many characters of the quoted text are actually sent to the AI. Anything past this is cut off. Raise if you quote long passages and want the AI to see all of it; lower if your model has a small context window or you want to save tokens on big selections.',
  },
  search: {
    searxng_url:
      "Where Thuki's local search engine (SearXNG) is running. SearXNG sends your query to Google, Bing, etc. and brings the results back. Keep this on 127.0.0.1; pointing it at a remote host leaks every search query and breaks Thuki's sandbox isolation.",
    reader_url:
      "Where Thuki's local web-page reader is running. The reader opens promising URLs, strips out ads, menus, and scripts, and hands the clean text back so the AI can read it. Keep this on 127.0.0.1; a remote reader could fetch arbitrary URLs from a host with access to private networks.",
    searxng_max_results:
      'How many results SearXNG returns for each query, before Thuki ranks them and picks the best ones to read. Raise for wider coverage (more candidate URLs to pick from); lower for faster, narrower searches.',
    max_iterations:
      'How many rounds of searching the AI is allowed to do for a single question. If the first round of results is not enough, the AI generates new queries and tries again. Raise for hard, multi-step questions that need more digging; lower for faster answers and fewer tokens.',
    top_k_urls:
      'How many web pages Thuki actually opens and reads after picking the most promising ones from the search results. Raise to give the AI more sources to pull facts from in its answer; lower for faster searches with less to read.',
    search_timeout_s:
      'How long (in seconds) Thuki waits for SearXNG to come back with search results before giving up on a single query. Raise this if you have a slow internet connection. Lowering it only causes searches to give up before they would have succeeded.',
    reader_per_url_timeout_s:
      'How long (in seconds) Thuki waits for one single web page to load before giving up on it and moving on. Raise this for slow websites that take a while to respond. Lowering it just makes more pages get skipped.',
    reader_batch_timeout_s:
      'How long (in seconds) Thuki waits for the whole batch of pages it is reading in parallel to finish. Must be larger than the per-URL timeout; if it is not, Thuki automatically bumps it to per-URL + 5. Raise on slow connections so a few slow pages do not kill the whole batch.',
    judge_timeout_s:
      'How long (in seconds) Thuki waits for the AI to decide whether the search results are good enough to answer your question. Raise this if your local AI model is slow on your hardware. Lowering it only causes the judging step to give up early.',
    router_timeout_s:
      'How long (in seconds) Thuki waits for the AI to decide whether your question even needs a web search and to plan the first queries. Raise this if your local AI model is slow on your hardware. Lowering it only causes the planning step to give up early.',
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
