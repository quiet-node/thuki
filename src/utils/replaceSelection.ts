import { invoke } from '@tauri-apps/api/core';
import type { Message } from '../hooks/useModel';

/**
 * Writes a `/rewrite` or `/refine` result back into the source app, replacing
 * the user's selection. Resolves to `true` when the backend confirms it wrote
 * the text; a `'skipped'` outcome (nothing safe to write into) or any IPC
 * failure resolves to `false`. Failures are intentionally swallowed: the
 * result still lives in chat and the user can copy it manually.
 */
export async function replaceSelection(text: string): Promise<boolean> {
  try {
    const outcome = await invoke<'replaced' | 'skipped'>('replace_selection', {
      text,
    });
    return outcome === 'replaced';
  } catch {
    return false;
  }
}

/**
 * Whether a completed turn should auto-replace the source selection: the
 * setting is on, the turn came from a replaceable command (`/rewrite` or
 * `/refine`), it produced content, and the user had selected text to replace.
 */
export function shouldAutoReplace(
  autoReplace: boolean,
  assistantMsg: Pick<Message, 'replaceCommand' | 'content'>,
  userMsg: Pick<Message, 'quotedText'>,
): boolean {
  return Boolean(
    autoReplace &&
    assistantMsg.replaceCommand &&
    assistantMsg.content &&
    userMsg.quotedText,
  );
}
