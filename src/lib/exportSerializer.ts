/**
 * Chat session export serialisers.
 *
 * Three outputs:
 *
 * - {@link serializeForFile}: self-contained Markdown artefact. Includes a
 *   YAML frontmatter block, the conditional customised system prompt, and
 *   inline base64 data URIs for screenshots. Suitable for archival, GitHub,
 *   Notion, Obsidian, or pasting back into another LLM.
 * - {@link serializeForFileAsText}: header block plus a plain text body
 *   (no Markdown markers, no YAML, no inline base64). The exported file
 *   is what TextEdit shows when the user picks "Save as Plain text…": a
 *   human-readable transcript with image markers instead of pixel data.
 * - {@link serializeForClipboard}: body-only Markdown. Frontmatter is
 *   stripped and screenshots are replaced with a textual placeholder so
 *   pasting into Slack, Discord, or a plain editor stays readable and does
 *   not detonate multi-megabyte base64 payloads on the clipboard.
 *
 * All three functions are pure with respect to the date and the
 * caller-provided configuration. The caller injects `now: Date` so tests
 * can assert deterministic output and so the export captures a single
 * coherent moment instead of drifting across nested `new Date()` calls.
 *
 * Markdown emission escapes untrusted strings (model name, search
 * source title, search source URL) at the boundary: search sources
 * with `javascript:` or otherwise non-http(s) URLs are degraded to
 * plain text so an exported file opened in a third-party Markdown
 * viewer cannot turn into a clickable hostile link.
 */

import { convertFileSrc } from '@tauri-apps/api/core';
import type { Message } from '../hooks/useOllama';

/** Configuration relevant to a file export. */
export interface FileExportContext {
  /**
   * Slug of the model currently selected at export time. Used only as a
   * fallback for assistant messages that have no `modelName` attribution
   * (legacy conversations loaded from pre-attribution history rows).
   * `undefined` is treated identically to `null`.
   */
  readonly fallbackModel: string | null | undefined;
}

/**
 * Returns the default filename suggested in the native save dialog.
 *
 * Format: `thuki-chat-YYYY-MM-DD-HHMM.<ext>`. Local timezone (matches
 * what the user perceives as "now"). No slug from the first user
 * message so a privacy-sensitive snippet does not become visible in
 * Finder / Spotlight. `ext` defaults to `'md'` so callers that haven't
 * been updated still get the canonical Markdown extension.
 */
export function defaultExportFilename(
  now: Date,
  ext: 'md' | 'txt' = 'md',
): string {
  const yyyy = now.getFullYear();
  const mm = pad2(now.getMonth() + 1);
  const dd = pad2(now.getDate());
  const hh = pad2(now.getHours());
  const mi = pad2(now.getMinutes());
  return `thuki-chat-${yyyy}-${mm}-${dd}-${hh}${mi}.${ext}`;
}

/**
 * Resolves a screenshot file path to a `data:` URI for inline embedding.
 *
 * Uses the Tauri asset protocol (`convertFileSrc`) so the renderer can
 * fetch the file without any new IPC. Reads via `fetch` + `FileReader`
 * because both are first-class browser APIs in the webview and require
 * no additional Tauri plugin (`fs:`) scope.
 *
 * Surfaced as a hook so tests can stub it without driving the asset
 * protocol or the network at all. The default implementation is
 * exported for production wiring.
 */
export type ImageLoader = (path: string) => Promise<string>;

export const defaultImageLoader: ImageLoader = (path) => pathToDataUri(path);

async function pathToDataUri(path: string): Promise<string> {
  const url = convertFileSrc(path);
  const response = await fetch(url);
  const blob = await response.blob();
  return await blobToDataUri(blob);
}

function blobToDataUri(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result;
      if (typeof result === 'string') {
        resolve(result);
      } else {
        reject(new Error('FileReader did not return a string data URI'));
      }
    };
    reader.onerror = () =>
      reject(reader.error ?? new Error('FileReader error'));
    reader.readAsDataURL(blob);
  });
}

/**
 * Serialises an entire conversation into a self-contained Markdown file.
 *
 * Asynchronous because screenshots are read from disk and base64-encoded.
 * Image read failures fall back to a textual placeholder so a single
 * broken file path never aborts the whole export. Image loads run in
 * parallel via `Promise.all` so the per-message latency is bounded by
 * the slowest image rather than the sum of all images.
 */
export async function serializeForFile(
  messages: readonly Message[],
  ctx: FileExportContext,
  now: Date,
  loadImage: ImageLoader = defaultImageLoader,
): Promise<string> {
  const frontmatter = buildFrontmatter(messages, ctx, now);
  const body = await buildBody(messages, loadImage);
  return `${frontmatter}\n${body}`;
}

/**
 * Serialises an entire conversation into a plain text file. No Markdown
 * markers, no YAML, no base64. Screenshots are replaced with textual
 * markers so the exported file is readable as-is in TextEdit or any
 * other plain-text editor without paging through megabytes of pixel
 * data.
 *
 * Includes a short labelled header (model, export timestamp, message
 * count) so the file is self-describing without imposing YAML syntax
 * on a text reader.
 */
export function serializeForFileAsText(
  messages: readonly Message[],
  ctx: FileExportContext,
  now: Date,
): string {
  const header = buildTextHeader(messages, ctx, now);
  const body = buildBodyTextOnly(messages);
  return body.length > 0 ? `${header}\n\n${body}` : `${header}\n`;
}

/**
 * Serialises an entire conversation into clipboard-friendly Markdown.
 *
 * No frontmatter (would surface as noisy text when pasted into chat
 * apps), no base64 images (multi-megabyte clipboards crash paste flows
 * in Slack/Discord). Image messages render as a textual marker so the
 * context that a screenshot existed is preserved.
 */
export function serializeForClipboard(messages: readonly Message[]): string {
  return buildBodyMarkdownTextOnly(messages);
}

function buildFrontmatter(
  messages: readonly Message[],
  ctx: FileExportContext,
  now: Date,
): string {
  return [
    '---',
    `app: ${yamlQuote('Thuki')}`,
    `model: ${yamlQuote(pickModel(messages, ctx.fallbackModel))}`,
    `exported_at: ${yamlQuote(isoLocal(now))}`,
    `message_count: ${messages.length}`,
    '---',
  ].join('\n');
}

function buildTextHeader(
  messages: readonly Message[],
  ctx: FileExportContext,
  now: Date,
): string {
  return [
    'Thuki chat export',
    `Model: ${pickModel(messages, ctx.fallbackModel)}`,
    `Exported: ${isoLocal(now)}`,
    `Messages: ${messages.length}`,
  ].join('\n');
}

async function buildBody(
  messages: readonly Message[],
  loadImage: ImageLoader,
): Promise<string> {
  const sections = await Promise.all(
    messages.map((message) => renderMessage(message, loadImage)),
  );
  return sections.join('\n\n---\n\n').concat(sections.length > 0 ? '\n' : '');
}

function buildBodyMarkdownTextOnly(messages: readonly Message[]): string {
  const sections = messages.map(renderMessageTextOnly);
  return sections.join('\n\n---\n\n').concat(sections.length > 0 ? '\n' : '');
}

function buildBodyTextOnly(messages: readonly Message[]): string {
  const sections = messages.map(renderMessagePlainText);
  return sections
    .join('\n\n----------\n\n')
    .concat(sections.length > 0 ? '\n' : '');
}

async function renderMessage(
  message: Message,
  loadImage: ImageLoader,
): Promise<string> {
  const parts: string[] = [`## ${roleLabel(message)}`];
  const quote = renderQuote(message);
  if (quote) parts.push(quote);
  if (message.thinkingContent) {
    parts.push(renderThinking(message.thinkingContent));
  }
  if (message.content) parts.push(message.content);
  const images = await renderImages(message, loadImage);
  if (images) parts.push(images);
  const sources = renderSources(message);
  if (sources) parts.push(sources);
  return parts.join('\n\n');
}

function renderMessageTextOnly(message: Message): string {
  const parts: string[] = [`## ${roleLabel(message)}`];
  const quote = renderQuote(message);
  if (quote) parts.push(quote);
  if (message.thinkingContent) {
    parts.push(renderThinking(message.thinkingContent));
  }
  if (message.content) parts.push(message.content);
  const imageMarkers = renderImagesAsMarkers(message);
  if (imageMarkers) parts.push(imageMarkers);
  const sources = renderSources(message);
  if (sources) parts.push(sources);
  return parts.join('\n\n');
}

function renderMessagePlainText(message: Message): string {
  const parts: string[] = [`${roleLabel(message)}:`];
  if (message.quotedText) {
    parts.push(renderQuotePlain(message.quotedText));
  }
  if (message.content) parts.push(message.content);
  const imageMarkers = renderImagesAsPlainMarkers(message);
  if (imageMarkers) parts.push(imageMarkers);
  const sources = renderSourcesPlain(message);
  if (sources) parts.push(sources);
  return parts.join('\n\n');
}

function roleLabel(message: Message): string {
  if (message.role === 'user') return 'User';
  return message.modelName ? `Assistant (${message.modelName})` : 'Assistant';
}

function renderQuote(message: Message): string | null {
  if (!message.quotedText) return null;
  return message.quotedText
    .split('\n')
    .map((line) => `> ${line}`)
    .join('\n');
}

function renderQuotePlain(quoted: string): string {
  return quoted
    .split('\n')
    .map((line) => `  ${line}`)
    .join('\n');
}

function renderThinking(thinking: string): string {
  return `<details>\n<summary>Thinking</summary>\n\n${thinking}\n\n</details>`;
}

async function renderImages(
  message: Message,
  loadImage: ImageLoader,
): Promise<string | null> {
  if (!message.imagePaths || message.imagePaths.length === 0) return null;
  const rendered = await Promise.all(
    message.imagePaths.map((path) => renderSingleImage(path, loadImage)),
  );
  return rendered.join('\n\n');
}

async function renderSingleImage(
  path: string,
  loadImage: ImageLoader,
): Promise<string> {
  try {
    const dataUri = await loadImage(path);
    return `![Screenshot](${dataUri})`;
  } catch {
    return `_[Screenshot unavailable: ${basename(path)}]_`;
  }
}

function renderImagesAsMarkers(message: Message): string | null {
  if (!message.imagePaths || message.imagePaths.length === 0) return null;
  return message.imagePaths
    .map((path) => `_[Screenshot: ${basename(path)}]_`)
    .join('\n\n');
}

function renderImagesAsPlainMarkers(message: Message): string | null {
  if (!message.imagePaths || message.imagePaths.length === 0) return null;
  return message.imagePaths
    .map((path) => `[Screenshot: ${basename(path)}]`)
    .join('\n');
}

function renderSources(message: Message): string | null {
  const sources = message.searchSources;
  if (!sources || sources.length === 0) return null;
  const lines = ['**Sources** (`/search`):'];
  sources.forEach((source, index) => {
    const title = source.title || source.url;
    lines.push(formatMarkdownSourceLine(index + 1, title, source.url));
  });
  return lines.join('\n');
}

function renderSourcesPlain(message: Message): string | null {
  const sources = message.searchSources;
  if (!sources || sources.length === 0) return null;
  const lines = ['Sources (/search):'];
  sources.forEach((source, index) => {
    const title = source.title || source.url;
    lines.push(`${index + 1}. ${title} - ${source.url}`);
  });
  return lines.join('\n');
}

/**
 * Renders one ordered-list line of the **Sources** block. Untrusted
 * title characters are backslash-escaped; the URL scheme is validated
 * against an http(s)-only allowlist so a search source carrying a
 * `javascript:` URL cannot land in the exported file as a clickable
 * link in a third-party Markdown viewer (Streamdown + rehype-sanitize
 * protect Thuki's own renderer but the exported file is consumed
 * elsewhere).
 *
 * Safe URLs are wrapped in `<...>` angle brackets so URLs containing
 * `(` `)` parse correctly per CommonMark. Unsafe URLs fall through to
 * a plain-text rendering that quotes the raw URL after the title so no
 * information is lost while no anchor element is generated.
 */
export function formatMarkdownSourceLine(
  index: number,
  title: string,
  url: string,
): string {
  const escapedTitle = escapeMarkdownLinkText(title);
  if (isSafeHttpUrl(url) && !containsAngleBracket(url)) {
    return `${index}. [${escapedTitle}](<${url}>)`;
  }
  return `${index}. ${escapedTitle} (link omitted: ${url})`;
}

/**
 * Returns `true` for `http://` or `https://` URLs only. Everything
 * else - including `javascript:`, `data:`, `file:`, `vbscript:`,
 * bare strings, and URLs with a CR/LF in them - is rejected.
 */
export function isSafeHttpUrl(value: string): boolean {
  if (/[\r\n\t]/.test(value)) return false;
  return /^https?:\/\//i.test(value);
}

function containsAngleBracket(value: string): boolean {
  return value.includes('<') || value.includes('>');
}

/**
 * Backslash-escapes the CommonMark link-text metacharacters so a title
 * containing `]`, `[`, or `\` cannot truncate or repoint the rendered
 * link. Also escapes leading/trailing whitespace artefacts that would
 * otherwise produce a soft-break.
 */
export function escapeMarkdownLinkText(value: string): string {
  return value
    .replace(/\\/g, '\\\\')
    .replace(/\[/g, '\\[')
    .replace(/\]/g, '\\]');
}

/**
 * YAML double-quoted string encoding. Backslash and double-quote are
 * escaped, ASCII control characters are converted to their `\xHH`
 * escape, and a literal newline becomes `\n`. Wrapping the result in
 * double quotes produces a string that any conformant YAML 1.1/1.2
 * parser (Obsidian, Jekyll, Hugo, Pandoc) decodes back to the
 * original characters.
 */
export function yamlQuote(value: string): string {
  let escaped = '';
  for (const ch of value) {
    const code = ch.charCodeAt(0);
    if (ch === '\\') {
      escaped += '\\\\';
    } else if (ch === '"') {
      escaped += '\\"';
    } else if (ch === '\n') {
      escaped += '\\n';
    } else if (ch === '\r') {
      escaped += '\\r';
    } else if (ch === '\t') {
      escaped += '\\t';
    } else if (code < 0x20 || code === 0x7f) {
      escaped += `\\x${code.toString(16).padStart(2, '0')}`;
    } else {
      escaped += ch;
    }
  }
  return `"${escaped}"`;
}

function pickModel(
  messages: readonly Message[],
  fallback: string | null | undefined,
): string {
  for (const message of messages) {
    if (message.role === 'assistant' && message.modelName) {
      return message.modelName;
    }
  }
  return fallback ?? 'unknown';
}

function pad2(value: number): string {
  return String(value).padStart(2, '0');
}

function isoLocal(date: Date): string {
  const offsetMinutes = -date.getTimezoneOffset();
  const sign = offsetMinutes >= 0 ? '+' : '-';
  const absOffset = Math.abs(offsetMinutes);
  const offsetH = pad2(Math.floor(absOffset / 60));
  const offsetM = pad2(absOffset % 60);
  return `${date.getFullYear()}-${pad2(date.getMonth() + 1)}-${pad2(
    date.getDate(),
  )}T${pad2(date.getHours())}:${pad2(date.getMinutes())}:${pad2(
    date.getSeconds(),
  )}${sign}${offsetH}:${offsetM}`;
}

function basename(path: string): string {
  const slash = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  return slash === -1 ? path : path.slice(slash + 1);
}
