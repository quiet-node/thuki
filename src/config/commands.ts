/**
 * Registry of all slash commands supported by the ask bar.
 *
 * Each entry drives both the CommandSuggestion autocomplete UI and the
 * submit-time parser in App.tsx. Adding a command here is sufficient:
 * no other registration is needed.
 */

export interface Command {
  /** The slash trigger, e.g. "/screen". Must start with "/". */
  readonly trigger: string;
  /** Short label shown in the suggestion row. */
  readonly label: string;
  /** One-line description shown as muted subtext in the suggestion row. */
  readonly description: string;
  /** Prompt template with $INPUT / $LANG placeholders. Absent for non-template commands. */
  readonly promptTemplate?: string;
}

export const COMMANDS: readonly Command[] = [
  {
    trigger: '/screen',
    label: '/screen',
    description: 'Capture your screen and include it as context',
  },
  {
    trigger: '/think',
    label: '/think',
    description: 'Think deeply before answering',
  },
  {
    trigger: '/translate',
    label: '/translate',
    description: 'Translate text to another language',
    promptTemplate:
      'You are a translation assistant. Translate the following text to the specified target language. The user may specify the target language by its full name (e.g., "Vietnamese"), ISO code (e.g., "vi", "vie"), abbreviation, or informal shorthand. Interpret the language identifier flexibly and use your best judgment. If no target language is specified: translate to English if the text is non-English, or to Vietnamese if it is already in English. Output only the translation with no commentary or explanation.\n\nTarget language: $LANG\n\nText: $INPUT',
  },
  {
    trigger: '/rewrite',
    label: '/rewrite',
    description: 'Rewrite text for clarity and flow',
    promptTemplate:
      'Please help rewrite the text below so it reads naturally and smoothly. Make it clear, easy to understand, and easy to follow. No icons, no em dashes. Please output only the rewritten text.\n\nText: $INPUT',
  },
  {
    trigger: '/tldr',
    label: '/tldr',
    description: 'Summarize text in 1-3 sentences',
    promptTemplate:
      "Summarize the following text into a TL;DR. Capture the core message in 1-3 short, direct sentences. Focus on what matters most: the main point, the key decision, or the critical takeaway. Skip background details, qualifications, and anything that isn't essential to understanding the gist. Output only the summary.\n\nText: $INPUT",
  },
  {
    trigger: '/refine',
    label: '/refine',
    description: 'Fix grammar, spelling, and punctuation',
    promptTemplate:
      'Refine the following text by correcting grammar, spelling, punctuation, and awkward phrasing. Keep the original tone, voice, and meaning intact. Do not restructure paragraphs, add new ideas, or remove content. If a sentence is grammatically correct but stylistically rough, smooth it lightly without changing the intent. Output only the refined text.\n\nText: $INPUT',
  },
  {
    trigger: '/bullets',
    label: '/bullets',
    description: 'Extract key points as a bullet list',
    promptTemplate:
      'Extract the key points from the following text as a bulleted list. Each bullet should be a concise, self-contained statement that captures one distinct idea. Order them by importance or logical sequence, whichever fits better. Leave out filler, repetition, and transitional fluff. Aim for bullets that someone could scan in 10 seconds and walk away informed.\n\nText: $INPUT',
  },
  {
    trigger: '/action',
    label: '/action',
    description: 'Extract action items as a checklist',
    promptTemplate:
      'Extract every action item, task, commitment, and follow-up from the following text. Present them as a checklist. Each item should state: who is responsible (if mentioned), what needs to be done, and any deadline or timeframe (if mentioned). If no owner is specified, phrase the item as a general task. Order by urgency or sequence when possible. Ignore observations, opinions, and background that do not imply a concrete next step.\n\nText: $INPUT',
  },
] as const;

/**
 * Sentinel image-path value used as a loading placeholder while the
 * /screen capture is in flight. ChatBubble detects this value and
 * renders a branded screen-capture loading tile instead of a broken image.
 */
export const SCREEN_CAPTURE_PLACEHOLDER = 'blob:screen-capture-loading';

/**
 * Builds a fully composed prompt from a utility command's template.
 *
 * Input resolution (selected text primary, typed text fallback):
 * 1. Selected text present, no typed text: selected text is $INPUT.
 * 2. No selected text, typed text present: typed text is $INPUT.
 * 3. Both present: selected text is $INPUT, typed text appended as instruction.
 *
 * For /translate, the first word of strippedMessage is treated as the target
 * language identifier. The model interprets it flexibly (full name, ISO code,
 * abbreviation). If the language word is the only typed content and there is
 * no selected text, returns null (no input to translate).
 *
 * Returns null if the command has no template, is unknown, or input is empty.
 */
export function buildPrompt(
  trigger: string,
  strippedMessage: string,
  selectedText?: string,
): string | null {
  const cmd = COMMANDS.find((c) => c.trigger === trigger);
  if (!cmd?.promptTemplate) return null;

  const typed = strippedMessage.trim();
  const selected = selectedText?.trim() ?? '';

  let lang = '';
  let typedRemainder = typed;

  if (trigger === '/translate' && typed) {
    const spaceIdx = typed.indexOf(' ');
    if (spaceIdx === -1) {
      // Single word: treat as language code only.
      lang = typed;
      typedRemainder = '';
    } else {
      lang = typed.slice(0, spaceIdx);
      typedRemainder = typed.slice(spaceIdx + 1).trim();
    }
  }

  // Resolve $INPUT.
  let input: string;
  if (selected && typedRemainder) {
    input = `${selected}\n\n[Additional instruction]: ${typedRemainder}`;
  } else if (selected) {
    input = selected;
  } else if (typedRemainder) {
    input = typedRemainder;
  } else {
    return null;
  }

  return cmd.promptTemplate.replace('$LANG', lang).replace('$INPUT', input);
}
