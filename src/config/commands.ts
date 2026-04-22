/**
 * Registry of all slash commands supported by the ask bar.
 *
 * Each entry drives both the CommandSuggestion autocomplete UI and the
 * submit-time parser in App.tsx. Adding a command here is sufficient:
 * no other registration is needed.
 */

export interface CommandDocs {
  /** Short paragraph used as the section opener in generated docs. */
  readonly summary: string;
  /** Usage string shown in generated docs. */
  readonly usage: string;
  /** Human-facing examples rendered as markdown bullets. */
  readonly examples: readonly string[];
  /** Main behavior description for generated docs. */
  readonly behavior: string;
  /** Optional composition note for generated docs. */
  readonly composability?: string;
  /** Optional limit note for generated docs. */
  readonly limit?: string;
  /** Optional permission note for generated docs. */
  readonly permission?: string;
  /** Optional language-format note for generated docs. */
  readonly languageFormat?: string;
  /** Optional default-behavior note for generated docs. */
  readonly defaultBehavior?: string;
}

export interface CommandPromptHelp {
  /** Short model-facing summary for the generated prompt appendix. */
  readonly summary: string;
  /** Conservative guidance on when this command should be mentioned. */
  readonly whenToSuggest?: string;
  /** Optional composition guidance for the prompt appendix. */
  readonly composition?: string;
  /** Optional limits or caveats for the prompt appendix. */
  readonly limit?: string;
}

export interface Command {
  /** The slash trigger, e.g. "/screen". Must start with "/". */
  readonly trigger: string;
  /** Short label shown in the suggestion row. */
  readonly label: string;
  /** One-line description shown as muted subtext in the suggestion row. */
  readonly description: string;
  /** Human-facing docs metadata used to generate docs/commands.md. */
  readonly docs: CommandDocs;
  /** Model-facing metadata used to generate the slash-command prompt appendix. */
  readonly promptHelp: CommandPromptHelp;
  /** Prompt template with $INPUT / $LANG placeholders. Absent for non-template commands. */
  readonly promptTemplate?: string;
}

export const COMMANDS: readonly Command[] = [
  {
    trigger: '/search',
    label: '/search',
    description: 'Agentic web search: iterative reasoning & cited synthesis',
    docs: {
      summary:
        'Runs agentic web search and answers from live sources with citations.',
      usage: '/search <question>',
      examples: [
        '`/search who owns Figma now?`: searches live sources for a current answer',
        '`/search latest React 19 release notes`: retrieves recent release information from the web',
      ],
      behavior:
        "Routes the message through Thuki's local search pipeline instead of plain chat. Answers are grounded in retrieved web sources and typically include inline citations plus a Sources footer.",
      limit:
        'Requires the search sandbox to be running. Use it for current, changing, or cutoff-sensitive information.',
    },
    promptHelp: {
      summary: 'agentic web search for current or cutoff-sensitive questions.',
      whenToSuggest:
        'Mention this when the user asks for current web information, live prices, recent releases, current ownership, or facts likely newer than the model cutoff.',
      limit:
        'Do not claim to have searched the web without `/search`. `/search` requires the local search sandbox.',
    },
  },
  {
    trigger: '/screen',
    label: '/screen',
    description: 'Capture your screen and include it as context',
    docs: {
      summary:
        'Captures your screen and attaches it as context for the current message.',
      usage: '/screen [optional message]',
      examples: [
        '`/screen`: sends a screenshot with no additional message',
        '`/screen what is this error?`: attaches a screenshot and asks a question about it',
      ],
      behavior:
        "The screenshot is taken when you submit the message. Thuki's own window is excluded from the capture, and the image appears in your message bubble like a pasted screenshot.",
      composability:
        '`/screen` can combine with `/think` and utility commands. For example, `/screen /rewrite` captures the screen and rewrites whatever text the model can see.',
      limit:
        'One `/screen` capture per message. You may also attach up to 3 images manually for a total of 4 images per message.',
      permission:
        'Requires Screen Recording permission. If denied, Thuki cannot capture the screen until access is granted in System Settings.',
    },
    promptHelp: {
      summary: 'capture current screen and attach it as image context.',
      composition:
        'Can combine with `/think` and utility commands in the same message.',
      limit:
        'One `/screen` capture per message and it requires Screen Recording permission.',
    },
  },
  {
    trigger: '/think',
    label: '/think',
    description: 'Think deeply before answering',
    docs: {
      summary: 'Enables extended reasoning before the model responds.',
      usage: '/think [optional message or highlighted text]',
      examples: [
        '`/think` with highlighted text: reasons through the selected content',
        '`/think what are the tradeoffs of a monorepo vs polyrepo?`: asks a question with deep reasoning enabled',
      ],
      behavior:
        "A collapsible Thinking block appears above the response showing the model's reasoning chain. The final answer appears below it as normal.",
      composability:
        '`/think` works with `/screen` and all utility commands. For example, `/think /tldr` summarizes with extended reasoning enabled.',
    },
    promptHelp: {
      summary: 'enable extended reasoning before answering.',
      composition: 'Can combine with `/screen` and utility commands.',
    },
  },
  {
    trigger: '/translate',
    label: '/translate',
    description: 'Translate text to another language',
    docs: {
      summary: 'Translates text to another language.',
      usage: '/translate [language] [text] or /translate with highlighted text',
      examples: [
        '`/translate` with highlighted text: auto-detects the source language and translates it',
        '`/translate ja` with highlighted text: translates highlighted text to Japanese',
        '`/translate Spanish meeting notes here`: translates typed text to Spanish',
      ],
      behavior:
        'Outputs only the translation with no commentary or explanation.',
      languageFormat:
        'The target language can be a full name (`French`), ISO code (`fr`, `fra`), or common shorthand.',
      defaultBehavior:
        'If no language is specified, non-English input translates to English and English input translates to Vietnamese.',
    },
    promptHelp: {
      summary: 'translate selected or typed text to requested language.',
      limit:
        'If no language is given, non-English input goes to English and English input goes to Vietnamese.',
    },
    promptTemplate:
      'You are a translation assistant. Translate the following text to the specified target language. The user may specify the target language by its full name (e.g., "Vietnamese"), ISO code (e.g., "vi", "vie"), abbreviation, or informal shorthand. Interpret the language identifier flexibly and use your best judgment. If no target language is specified: translate to English if the text is non-English, or to Vietnamese if it is already in English. Output only the translation with no commentary or explanation.\n\nTarget language: $LANG\n\nText: $INPUT',
  },
  {
    trigger: '/rewrite',
    label: '/rewrite',
    description: 'Rewrite text for clarity and flow',
    docs: {
      summary: 'Rewrites text to read more naturally and clearly.',
      usage: '/rewrite [text] or /rewrite with highlighted text',
      examples: [
        '`/rewrite` with highlighted text: rewrites the selected text',
        '`/rewrite so basically what happened was i was trying to fix the bug`: rewrites typed text for clarity',
      ],
      behavior:
        'Preserves the original meaning while improving flow and readability. Outputs only the rewritten text.',
    },
    promptHelp: {
      summary: 'rewrite text for clarity and flow.',
    },
    promptTemplate:
      'Please help rewrite the text below so it reads naturally and smoothly. Make it clear, easy to understand, and easy to follow. No icons, no em dashes. Please output only the rewritten text.\n\nText: $INPUT',
  },
  {
    trigger: '/tldr',
    label: '/tldr',
    description: 'Summarize text in 1-3 sentences',
    docs: {
      summary: 'Summarizes text into 1-3 short, direct sentences.',
      usage: '/tldr [text] or /tldr with highlighted text',
      examples: [
        '`/tldr` with highlighted text: summarizes the selected content',
        '`/tldr [paste a long article]`: summarizes typed or pasted text',
      ],
      behavior:
        'Captures the core message, key decision, or critical takeaway. Skips background detail and qualifications.',
    },
    promptHelp: {
      summary: 'summarize text in 1-3 short direct sentences.',
    },
    promptTemplate:
      "Summarize the following text into a TL;DR. Capture the core message in 1-3 short, direct sentences. Focus on what matters most: the main point, the key decision, or the critical takeaway. Skip background details, qualifications, and anything that isn't essential to understanding the gist. Output only the summary.\n\nText: $INPUT",
  },
  {
    trigger: '/refine',
    label: '/refine',
    description: 'Fix grammar, spelling, and punctuation',
    docs: {
      summary:
        'Fixes grammar, spelling, and punctuation while preserving your voice.',
      usage: '/refine [text] or /refine with highlighted text',
      examples: [
        '`/refine` with highlighted text: corrects the selected text',
        '`/refine hey just wanted to follow up on the thing we discussed`: cleans up typed text',
      ],
      behavior:
        'Corrects errors and smooths rough phrasing without restructuring or adding new ideas. Your original tone and meaning stay intact.',
    },
    promptHelp: {
      summary:
        'fix grammar, spelling, punctuation, and rough phrasing while preserving tone.',
    },
    promptTemplate:
      'Refine the following text by correcting grammar, spelling, punctuation, and awkward phrasing. Keep the original tone, voice, and meaning intact. Do not restructure paragraphs, add new ideas, or remove content. If a sentence is grammatically correct but stylistically rough, smooth it lightly without changing the intent. Output only the refined text.\n\nText: $INPUT',
  },
  {
    trigger: '/bullets',
    label: '/bullets',
    description: 'Extract key points as a bullet list',
    docs: {
      summary: 'Extracts key points from text as a markdown bullet list.',
      usage: '/bullets [text] or /bullets with highlighted text',
      examples: [
        '`/bullets` with highlighted text: extracts key points from the selection',
        '`/bullets [paste meeting notes]`: extracts key points from typed or pasted content',
      ],
      behavior:
        'Each point is a concise, self-contained statement. Ordered by importance or logical sequence. Filler and repetition are removed. Output uses `- ` prefixed markdown bullets.',
    },
    promptHelp: {
      summary: 'extract key points as markdown bullets.',
    },
    promptTemplate:
      'Extract the key points from the following text as a bulleted list. Each item must begin with "- " (a hyphen followed by a space). Do not use numbered lists, plain paragraphs, headers, or any other formatting. Output only the bulleted list, nothing else.\n\nExample output format:\n- First key point\n- Second key point\n- Third key point\n\nEach bullet should be a concise, self-contained statement. Order by importance or logical sequence. Leave out filler and repetition.\n\nText: $INPUT',
  },
  {
    trigger: '/todos',
    label: '/todos',
    description: 'Extract to-do items as a checkbox list',
    docs: {
      summary:
        'Summarizes what a piece of text is about, then extracts every task, action item, and commitment as a markdown checkbox list.',
      usage: '/todos [text] or /todos with highlighted text',
      examples: [
        '`/todos` with highlighted text: summarizes and extracts to-dos from the selected text',
        '`/todos [paste a conversation or notes]`: processes typed or pasted content',
      ],
      behavior:
        'Responds in two parts: a short paragraph explaining the context and what is at stake, followed by a `- [ ]` checkbox list of all tasks. Each to-do includes who is responsible, plus any deadline or timeframe if mentioned.',
    },
    promptHelp: {
      summary: 'summarize context and extract tasks as markdown checkboxes.',
    },
    promptTemplate:
      'Read the following text and respond in two parts:\n\n**Part 1: Summary.** Write a short paragraph (3-5 sentences) explaining what this text is about. Cover: what the situation or topic is, who is involved, what the current state is, and why it matters or what is at stake. This should give someone who has not read the original text a clear picture of the context.\n\n**Part 2: To-dos.** List every task, action item, commitment, and follow-up from the text as a markdown checkbox list. Every single item MUST begin with "- [ ] " (hyphen, space, open bracket, space, close bracket, space). Do not use numbered lists, plain bullets, headers, or any other format for the list items.\n\nSeparate the two parts with a blank line. Do not add any headings or labels like "Summary:" or "To-dos:"; just write the paragraph, then the list.\n\nExample output format:\nThis is a paragraph explaining what the text is about, who is involved, and what the situation is. It gives enough context to understand why the tasks matter. It is clear and direct.\n\n- [ ] First task to complete\n- [ ] Second task to complete\n- [ ] Third task to complete\n\nFor each to-do item, include who is responsible (if mentioned), what needs to be done, and any deadline or timeframe (if mentioned). Order by urgency or sequence when possible.\n\nText: $INPUT',
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

  return cmd.promptTemplate.replace(/\$LANG|\$INPUT/g, (m) =>
    m === '$LANG' ? lang : input,
  );
}
