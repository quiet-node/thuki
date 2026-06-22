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
  'Use /extract to extract any text from images or screenshots',
  {
    text: 'OCR-supported commands (/extract, /translate, etc.) read images locally: no vision model needed ↗',
    url: 'https://github.com/quiet-node/thuki/blob/main/docs/ocr-commands.md',
  },
  'Highlight text in any app before summoning Thuki to include it as context',
  '/think makes Thuki reason step by step before answering, great for hard questions',
  '/search pulls live web results into the chat so answers stay current',
  '⌘W or Esc hides the window; Thuki keeps running in the background',
  'Drop an image onto the bar to attach it and ask questions about what you see',
  'Paste images from your clipboard directly; no need to save to disk first',
  'Click the chip icon to switch between any model you have installed',
  'The bookmark icon saves the full conversation so you can come back to it later',
  '/translate converts your selected text to any language you specify',
  'Click the clock icon to browse all your past conversations',
  'Highlight any text and type /rewrite to get a cleaner, better-flowing version without changing the meaning',
  '/tldr condenses any highlighted or pasted block of text into 1-3 sentences',
  '/refine fixes grammar, spelling, and punctuation in highlighted text while keeping your voice and tone',
  '/bullets turns highlighted text or a pasted block into a concise bullet list of key points',
  '/todos scans highlighted text or notes and pulls out every action item as a checkbox list',
  'Type / in the ask bar to see all available commands and pick one with Tab',
  {
    text: 'Slash commands have a full reference when you want the whole toolbox at a glance ↗',
    url: 'https://github.com/quiet-node/thuki/blob/main/docs/commands.md',
  },
  'Commands can combine in one message: try /screen /think to capture your screen and reason through it',
  'Paste an image and type /tldr to summarize its text using Vision OCR; no vision model needed',
  'Type /translate french with an attached image to translate printed text via Vision OCR, no vision model needed',
  'Everything runs locally on your Mac; your conversations never leave your machine',
  'Attach images to your messages for visual context; visit Settings to adjust the limit',
  'Turn on Keep Warm in Settings to skip the cold-load wait so your first reply is near-instant every time',
  'Keep Warm holds your active model ready in memory so there is no loading delay when you summon Thuki',
  'Set a release timer in Settings to keep your active model warm for a while, then free memory automatically',
  'Keep Warm auto-releases after your chosen timeout so it never holds memory longer than you need',
  'The green dot by your active model in Settings means it is live in memory and ready to respond instantly',
  'Keep Warm in Settings keeps your active model loaded between sessions so Thuki is always ready at full speed',
  'Set Keep Warm to -1 in Settings to keep your active model loaded indefinitely until you unload it yourself',
  'Click Unload now in Settings to free your model from memory the moment you are done with it',
  {
    text: 'The config reference shows every setting you can tune without guessing names ↗',
    url: 'https://github.com/quiet-node/thuki/blob/main/docs/configurations.md',
  },
  'Crank the Context Window slider in Settings up if Thuki is forgetting earlier parts of long conversations',
  'Lower the Context Window in Settings to reclaim memory if your Mac is running tight',
  'Doubling the context window roughly doubles the memory the KV cache needs; nudge it up gradually',
  'Each model has a trained context limit; values above it are silently clamped down to that max',
  'The default 16K context fits a long chat; raise it in Settings when you paste big documents or whole files',
  'Type a token count directly into the chip next to the Context Window slider for an exact value',
  'Settings shows your active model, whether it is warm, and its active context length at a glance',
  {
    text: 'Context Window can be tuned in Settings. Learn how in five minutes ↗',
    url: 'https://github.com/quiet-node/thuki/blob/main/docs/tuning-context-window.md',
  },
  {
    text: 'Agentic search can dig deeper than a quick web lookup when the answer needs trail-following ↗',
    url: 'https://github.com/quiet-node/thuki/blob/main/docs/agentic-search.md',
  },
  'The export icon in the chat header saves the current session as a Markdown file',
  'Click the export icon and pick Copy to clipboard to grab the whole conversation, ready to paste anywhere',
  'Exports include the model name, every message, /think reasoning, search sources, and inline screenshots',
  'After /rewrite or /refine, hit Replace to send the result back into your app, replacing your selection',
  'Turn on Auto-replace in Settings → Behavior so /rewrite and /refine results return to your app automatically',
  'Turn on Auto-close in Settings → Behavior to dismiss Thuki automatically once /rewrite or /refine is replaced',
  'Thuki ships its own AI engine, so it works out of the box: no account, no API key, nothing to install',
  'Download more models in Settings → Models → Discover; Staff picks are vetted and sized for your Mac',
  'Settings → Models → Library lists everything you have downloaded; set any one active or delete it',
  'Prefer your own Ollama install? Switch to it anytime in Settings → Models → Providers',
  'Pick a model with the Vision badge to ask about images directly, no OCR step needed',
  'The Always reasons badge means a model reasons before every reply; others wait for /think',
  'Each model shows its trained context window in Settings, so you can see how much it reads at once',
  {
    text: 'New to local models? The Models and providers guide covers downloading and switching ↗',
    url: 'https://github.com/quiet-node/thuki/blob/main/docs/models-and-providers.md',
  },
];
