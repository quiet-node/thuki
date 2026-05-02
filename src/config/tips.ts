/**
 * A tip is either a plain string or a `{ text, url }` pair where the entire
 * tip bar becomes clickable and opens `url` in the user's default browser
 * via the Tauri `open_url` command. Use the linked form when the tip
 * references a public resource (docs, repo) so DMG users without the
 * codebase on disk can still reach it.
 */
export type Tip = string | { text: string; url: string };

export const TIPS: readonly Tip[] = [
  'Use /screen to snap your display and attach it to the chat for visual context',
  'Highlight text in any app before summoning Thuki to include it as context',
  '/think makes Thuki reason step by step before answering, great for hard questions',
  '/search pulls live web results into the chat so answers stay current',
  '⌘W or Esc hides the window; Thuki keeps running in the background',
  'Drop an image onto the bar to attach it and ask questions about what you see',
  'Paste images from your clipboard directly; no need to save to disk first',
  'Click the chip icon to switch between any model you have installed in Ollama',
  'The bookmark icon saves the full conversation so you can come back to it later',
  '/translate converts your selected text to any language you specify',
  'Click the clock icon to browse all your past conversations',
  'Highlight any text and type /rewrite to get a cleaner, better-flowing version without changing the meaning',
  '/tldr condenses any highlighted or pasted block of text into 1-3 sentences',
  '/refine fixes grammar, spelling, and punctuation in highlighted text while keeping your voice and tone',
  '/bullets turns highlighted text or a pasted block into a concise bullet list of key points',
  '/todos scans highlighted text or notes and pulls out every action item as a checkbox list',
  'Type / in the ask bar to see all available commands and pick one with Tab',
  'Commands can combine in one message: try /screen /think to capture your screen and reason through it',
  'Everything runs locally through Ollama; your conversations never leave your machine',
  'Attach images to your messages for visual context; visit Settings to adjust the limit',
  'Turn on Keep Warm in Settings to skip the cold-load wait so your first reply is near-instant every time',
  'Keep Warm holds your active model ready in VRAM so there is no loading delay when you summon Thuki',
  'Set a release timer in Settings to keep your active model warm for a while, then free VRAM automatically',
  'Keep Warm auto-releases after your chosen timeout so it never holds GPU memory longer than you need',
  'The green dot next to your active model in Settings means it is live in VRAM and ready to respond instantly',
  'Keep Warm in Settings keeps your active model loaded between sessions so Thuki is always ready at full speed',
  'Set Keep Warm to -1 in Settings to keep your active model loaded indefinitely until you unload it yourself',
  'Click Unload now in Settings to free your model from VRAM the moment you are done with it',
  'Crank the Context Window slider in Settings up if Thuki is forgetting earlier parts of long conversations',
  'Lower the Context Window in Settings to reclaim VRAM if your GPU is running tight on memory',
  'Doubling the context window roughly doubles the VRAM the KV cache needs; nudge it up gradually',
  'Ollama caps the effective context to your model trained max, so values above that are silently clamped down',
  'The default 16K context fits a long chat; raise it in Settings when you paste big documents or whole files',
  'Type a token count directly into the chip next to the Context Window slider for an exact value',
  'Run `ollama ps` in a terminal to see which model is loaded, its size, and the active context length',
  {
    text: 'Context Window can be tuned in Settings. Learn how in five minutes ↗',
    url: 'https://github.com/quiet-node/thuki/blob/main/docs/tuning-context-window.md',
  },
];
