<!-- Generated from src/config/commands.ts by `bun run generate:commands`. Do not edit manually. -->

# Commands

Website: [thuki.app](https://www.thuki.app/)

Commands are written as whole-word `/` triggers anywhere in your message. Press `/` to open the command suggestion menu, then Tab to complete or Enter to select.

Commands can be combined when their behavior allows it. For example, `/screen /think` captures the screen and enables extended reasoning, while `/think /tldr` summarizes with reasoning enabled.

Commands that operate on text follow a consistent input priority:

1. **Highlighted text + no typed text:** highlighted text is the input
2. **No highlighted text + typed text after command:** typed text is the input
3. **Both present:** highlighted text is the primary input; typed text is appended as an additional instruction

This means you can highlight text anywhere on screen, summon Thuki with double-tap Control, type a command, and hit Enter without retyping the selected content.

## Image input on text-only models

`/extract`, `/tldr`, `/translate`, `/rewrite`, `/refine`, `/bullets`, `/todos`, and `/explain` pull the text out of attached images locally via macOS Vision OCR, so they work even when the active model has no vision capability. They read text, not scenes: describing a textless image still needs a vision model. Only plain submits and `/screen` alone send the image itself to the model. See [OCR-supported commands](./ocr-commands.md) for the full list and details.

## /search

Forces a live web look-up and answers with citations (zero-setup).

**Usage:** `/search <question>`

**Examples:**
- `/search who owns Figma now?`: searches live sources for a current answer
- `/search latest React 19 release notes`: retrieves recent release information from the web

**Behavior:** Forces engines-only web search even when Auto search is off or would skip. Re-fetches from the source rather than reusing SERP/page cache, so it is the way to say "look it up again" when a previous answer looked stale. Answers are grounded in retrieved sources with inline citations and a Sources footer. With Auto search on (Settings → Behavior, default), plain messages may also search when live facts are needed.

**Limit:** Use when you want a forced look-up, Auto search is off, a plain turn skipped the web, or you need a fresh re-check.

---

## /extract

Extracts all visible text from screenshots or attached images using macOS Vision OCR.

**Usage:** `/extract [optional message]`

**Examples:**
- `/extract` with an attached image: extracts all text from the image
- `/screen /extract`: captures the screen and extracts all visible text

**Behavior:** Text is extracted using the macOS Vision framework and returned verbatim in a code block. No prose or explanation is added. When multiple images are provided, each result is separated by a horizontal rule. Returns "[No text detected]" when no readable text is found.

**Composable:** `/extract` can combine with `/screen` to capture then extract in one step.

**Permission:** Uses the same Screen Recording permission as `/screen` when combined with it.

---

## /screen

Captures your screen and attaches it as context for the current message.

**Usage:** `/screen [optional message]`

**Examples:**
- `/screen`: sends a screenshot with no additional message
- `/screen what is this error?`: attaches a screenshot and asks a question about it

**Behavior:** The screenshot is taken when you submit the message. Thuki's own window is excluded from the capture, and the image appears in your message bubble like a pasted screenshot.

**Composable:** `/screen` can combine with `/think` and utility commands. For example, `/screen /rewrite` captures the screen and rewrites whatever text the model can see.

**Limit:** One `/screen` capture per message. You may also attach up to 3 images manually for a total of 4 images per message.

**Permission:** Requires Screen Recording permission. If denied, Thuki cannot capture the screen until access is granted in System Settings.

---

## /think

Enables extended reasoning before the model responds.

**Usage:** `/think [optional message or highlighted text]`

**Examples:**
- `/think` with highlighted text: reasons through the selected content
- `/think what are the tradeoffs of a monorepo vs polyrepo?`: asks a question with deep reasoning enabled

**Behavior:** A collapsible Reasoning block appears above the response showing the model's reasoning chain. The final answer appears below it as normal.

**Composable:** `/think` works with `/screen` and all utility commands. For example, `/think /tldr` summarizes with extended reasoning enabled.

---

## /translate

Translates text to another language.

**Usage:** `/translate [language] [text] or /translate with highlighted text`

**Examples:**
- `/translate` with highlighted text: auto-detects the source language and translates it
- `/translate ja` with highlighted text: translates highlighted text to Japanese
- `/translate Spanish meeting notes here`: translates typed text to Spanish

**Behavior:** Outputs only the translation with no commentary or explanation.

**Composable:** `/translate` works with attached images or `/screen`. Vision OCR extracts the text first; translation runs on the result. Omitting a target language defaults to Vietnamese.

**Language format:** The target language can be a full name (`French`), ISO code (`fr`, `fra`), or common shorthand.

**Default behavior:** If no language is specified, the text is translated to Vietnamese.

---

## /rewrite

Rewrites text to sound like natural, casual, everyday speech.

**Usage:** `/rewrite [text] or /rewrite with highlighted text`

**Examples:**
- `/rewrite` with highlighted text: rewrites the selected text
- `/rewrite so basically what happened was i was trying to fix the bug`: rewrites typed text in a natural, casual voice

**Behavior:** Rewrites text to sound like a fluent native speaker talking day to day: relaxed and casual by default, while keeping your meaning, personality, and point of view. It mirrors your original formatting instead of flattening it: Markdown headings, bold, lists, links, blockquotes, and code all stay, and only the text inside them is improved. Quoted lines, code, URLs, @mentions, #channels, and the emoji or expressive spellings you used are kept exactly as written. It fixes what reads awkwardly and leaves what already reads well alone, only stays formal when the original clearly is, and will not bolt a "we" voice onto an imperative or impersonal note. Outputs only the rewritten text. A Replace button on the result writes the rewritten text straight back into the app you were using, replacing your selection; turn on auto-replace in Settings to skip the button. Follow-up tweaks in the same chat, like asking for a longer or more formal version, keep the Replace button too.

**Composable:** `/rewrite` works with attached images or `/screen`. Vision OCR extracts the text first, then rewrites it.

---

## /tldr

Summarizes text into 1-3 short, direct sentences.

**Usage:** `/tldr [text] or /tldr with highlighted text`

**Examples:**
- `/tldr` with highlighted text: summarizes the selected content
- `/tldr [paste a long article]`: summarizes typed or pasted text

**Behavior:** Captures the core message, key decision, or critical takeaway. Skips background detail and qualifications.

**Composable:** `/tldr` works with attached images or `/screen`. Vision OCR extracts the text first, then summarizes it.

---

## /refine

Fixes grammar, spelling, and punctuation while preserving your voice.

**Usage:** `/refine [text] or /refine with highlighted text`

**Examples:**
- `/refine` with highlighted text: corrects the selected text
- `/refine hey just wanted to follow up on the thing we discussed`: cleans up typed text

**Behavior:** Corrects errors and smooths rough phrasing without restructuring or adding new ideas. Your original tone and meaning stay intact. A Replace button on the result writes the refined text straight back into the app you were using, replacing your selection; turn on auto-replace in Settings to skip the button. Follow-up tweaks in the same chat, like asking for a longer or more formal version, keep the Replace button too.

**Composable:** `/refine` works with attached images or `/screen`. Vision OCR extracts the text first, then refines it.

---

## /bullets

Extracts key points from text as a markdown bullet list.

**Usage:** `/bullets [text] or /bullets with highlighted text`

**Examples:**
- `/bullets` with highlighted text: extracts key points from the selection
- `/bullets [paste meeting notes]`: extracts key points from typed or pasted content

**Behavior:** Each point is a concise, self-contained statement. Ordered by importance or logical sequence. Filler and repetition are removed. Output uses `- ` prefixed markdown bullets.

**Composable:** `/bullets` works with attached images or `/screen`. Vision OCR extracts the text first, then extracts key points.

---

## /explain

Explains any concept, term, or code snippet in plain language, always with a concrete example.

**Usage:** `/explain [text] or /explain with highlighted text`

**Examples:**
- `/explain` with highlighted code: explains what the code does and why
- `/explain what is a closure?`: explains the concept with a concrete example
- `/explain JWT`: breaks down the term with a real-world analogy and example

**Behavior:** Outputs a brief explanation followed by at least one concrete example. Assumes no background knowledge. Skips jargon or defines it when unavoidable. No intro or sign-off.

**Composable:** `/explain` works with attached images or `/screen`. Vision OCR extracts the text first, then explains it.

---

## /todos

Summarizes what a piece of text is about, then extracts every task, action item, and commitment as a markdown checkbox list.

**Usage:** `/todos [text] or /todos with highlighted text`

**Examples:**
- `/todos` with highlighted text: summarizes and extracts to-dos from the selected text
- `/todos [paste a conversation or notes]`: processes typed or pasted content

**Behavior:** Responds in two parts: a short paragraph explaining the context and what is at stake, followed by a `- [ ]` checkbox list of all tasks. Each to-do includes who is responsible, plus any deadline or timeframe if mentioned.

**Composable:** `/todos` works with attached images or `/screen`. Vision OCR extracts the text first, then extracts to-dos.
