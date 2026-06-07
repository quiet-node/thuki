import type { JSX } from 'react';
import { useEffect, useMemo, useRef } from 'react';
import { LexicalComposer } from '@lexical/react/LexicalComposer';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import { PlainTextPlugin } from '@lexical/react/LexicalPlainTextPlugin';
import { ContentEditable } from '@lexical/react/LexicalContentEditable';
import { LexicalErrorBoundary } from '@lexical/react/LexicalErrorBoundary';
import { AutoFocusPlugin } from '@lexical/react/LexicalAutoFocusPlugin';
import { HistoryPlugin } from '@lexical/react/LexicalHistoryPlugin';
import { mergeRegister } from '@lexical/utils';
import { registerLexicalTextEntity } from '@lexical/text';
import {
  $createLineBreakNode,
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  COMMAND_PRIORITY_HIGH,
  KEY_ARROW_DOWN_COMMAND,
  KEY_ARROW_UP_COMMAND,
  KEY_ENTER_COMMAND,
  KEY_ESCAPE_COMMAND,
  KEY_TAB_COMMAND,
  PASTE_COMMAND,
} from 'lexical';
import { $createCommandNode, CommandNode } from './CommandNode';
import { getCommandMatch } from './commandMatch';

/** Keyboard handlers the host wires to the slash-command popover. */
export interface AskBarKeyHandlers {
  /** Enter without Shift: host decides complete-vs-submit. */
  onEnterKey: () => void;
  /** Arrow Down while the popover is open: move the highlight down. */
  onArrowDown: () => void;
  /** Arrow Up while the popover is open: move the highlight up. */
  onArrowUp: () => void;
  /** Tab while the popover is open: complete the highlighted command. */
  onTab: () => void;
  /** Escape while the popover is open: dismiss it. */
  onEscape: () => void;
}

interface LexicalAskBarInputProps {
  /** Controlled text value (the host owns the canonical query string). */
  value: string;
  /** Called when the editor's text content changes. */
  onValueChange: (value: string) => void;
  /** Fired once when the editor transitions from empty to non-empty. */
  onFirstKeystroke?: () => void;
  /** Placeholder shown while the editor is empty. */
  placeholder: string;
  /** Receives the underlying contentEditable element for focus management. */
  inputRef: React.RefObject<HTMLDivElement | null>;
  /** Class names applied to the contentEditable element. */
  contentEditableClassName: string;
  /** True while the slash-command popover is open (enables key interception). */
  suggestionsOpen: boolean;
  /** Popover keyboard handlers. */
  keyHandlers: AskBarKeyHandlers;
  /**
   * Called on paste with the clipboard payload. Return true if the host
   * consumed it (e.g. extracted image files) so the default text paste is
   * suppressed; false to let the editor paste text normally.
   */
  onPaste: (clipboard: DataTransfer | null) => boolean;
}

/** Surfaces internal Lexical errors instead of silently corrupting state. */
export function handleEditorError(error: Error): never {
  throw error;
}

/**
 * Two-way controlled-value bridge. Pushes editor edits up to the host and
 * external value changes (command completion, clearing on submit) down into
 * the editor. After any editor-originated edit the host value already equals
 * the editor text, so the down-sync is a no-op and the caret never jumps; it
 * only rewrites for genuine external changes.
 */
export function ValueSyncPlugin({
  value,
  onValueChange,
  onFirstKeystroke,
}: {
  value: string;
  onValueChange: (value: string) => void;
  onFirstKeystroke?: () => void;
}) {
  const [editor] = useLexicalComposerContext();
  const valueRef = useRef(value);
  valueRef.current = value;
  const onValueChangeRef = useRef(onValueChange);
  onValueChangeRef.current = onValueChange;
  const onFirstKeystrokeRef = useRef(onFirstKeystroke);
  onFirstKeystrokeRef.current = onFirstKeystroke;
  // True while the host→editor sync is rewriting the editor, so the editor→host
  // listener ignores that self-inflicted change. Without this guard the rewrite
  // echoes back as onValueChange and can fight the host's state during rapid
  // clear/restore cycles (e.g. submit-then-cancel).
  const applyingExternalRef = useRef(false);

  // Editor → host.
  useEffect(() => {
    return editor.registerUpdateListener(({ editorState }) => {
      if (applyingExternalRef.current) return;
      const text = editorState.read(() => $getRoot().getTextContent());
      const prev = valueRef.current;
      if (text === prev) return;
      if (prev.length === 0 && text.length > 0) {
        onFirstKeystrokeRef.current?.();
      }
      onValueChangeRef.current(text);
    });
  }, [editor]);

  // Host → editor.
  useEffect(() => {
    const current = editor
      .getEditorState()
      .read(() => $getRoot().getTextContent());
    if (current === value) return;
    applyingExternalRef.current = true;
    // `discrete: true` commits synchronously. Without it the rewrite is async,
    // so a rapid clear→restore (submit then cancel) can reorder: the cancel
    // reads a stale editor state, skips its rebuild, and the late clear wins,
    // leaving the input blank. Synchronous commit keeps the editor in lockstep
    // with the host value.
    editor.update(
      () => {
        const root = $getRoot();
        root.clear();
        const paragraph = $createParagraphNode();
        const lines = value.split('\n');
        lines.forEach((line, i) => {
          if (i > 0) paragraph.append($createLineBreakNode());
          if (line !== '') paragraph.append($createTextNode(line));
        });
        root.append(paragraph);
        paragraph.selectEnd();
      },
      { discrete: true },
    );
    applyingExternalRef.current = false;
  }, [editor, value]);

  return null;
}

/**
 * Wires the editor's runtime behavior: editable state, violet command-token
 * highlighting, popover keyboard interception, and image paste. Commands are
 * registered once and read the latest props through a ref so the handlers stay
 * stable while always seeing current popover state.
 */
export function BehaviorPlugin({
  suggestionsOpen,
  keyHandlers,
  onPaste,
}: {
  suggestionsOpen: boolean;
  keyHandlers: AskBarKeyHandlers;
  onPaste: (clipboard: DataTransfer | null) => boolean;
}) {
  const [editor] = useLexicalComposerContext();

  const latestRef = useRef({ suggestionsOpen, keyHandlers, onPaste });
  latestRef.current = { suggestionsOpen, keyHandlers, onPaste };

  useEffect(() => {
    return mergeRegister(
      ...registerLexicalTextEntity(
        editor,
        getCommandMatch,
        CommandNode,
        (node) => $createCommandNode(node.getTextContent()),
      ),
    );
  }, [editor]);

  useEffect(() => {
    return mergeRegister(
      editor.registerCommand(
        KEY_ENTER_COMMAND,
        (event) => {
          // Shift+Enter falls through to the default line break.
          if (event?.shiftKey) return false;
          event?.preventDefault();
          latestRef.current.keyHandlers.onEnterKey();
          return true;
        },
        COMMAND_PRIORITY_HIGH,
      ),
      editor.registerCommand(
        KEY_ARROW_DOWN_COMMAND,
        (event) => {
          if (!latestRef.current.suggestionsOpen) return false;
          event.preventDefault();
          latestRef.current.keyHandlers.onArrowDown();
          return true;
        },
        COMMAND_PRIORITY_HIGH,
      ),
      editor.registerCommand(
        KEY_ARROW_UP_COMMAND,
        (event) => {
          if (!latestRef.current.suggestionsOpen) return false;
          event.preventDefault();
          latestRef.current.keyHandlers.onArrowUp();
          return true;
        },
        COMMAND_PRIORITY_HIGH,
      ),
      editor.registerCommand(
        KEY_TAB_COMMAND,
        (event) => {
          if (!latestRef.current.suggestionsOpen) return false;
          event.preventDefault();
          latestRef.current.keyHandlers.onTab();
          return true;
        },
        COMMAND_PRIORITY_HIGH,
      ),
      editor.registerCommand(
        KEY_ESCAPE_COMMAND,
        () => {
          if (!latestRef.current.suggestionsOpen) return false;
          latestRef.current.keyHandlers.onEscape();
          return true;
        },
        COMMAND_PRIORITY_HIGH,
      ),
      editor.registerCommand(
        PASTE_COMMAND,
        (event) => {
          const clipboard = (event as ClipboardEvent).clipboardData ?? null;
          if (latestRef.current.onPaste(clipboard)) {
            event.preventDefault();
            return true;
          }
          return false;
        },
        COMMAND_PRIORITY_HIGH,
      ),
    );
  }, [editor]);

  return null;
}

/**
 * Lexical-backed replacement for the AskBar's textarea + highlight-mirror.
 * A single contentEditable means the caret is native and never drifts off the
 * rendered glyphs; command triggers highlight inline via a text-entity node.
 */
export function LexicalAskBarInput({
  value,
  onValueChange,
  onFirstKeystroke,
  placeholder,
  inputRef,
  contentEditableClassName,
  suggestionsOpen,
  keyHandlers,
  onPaste,
}: LexicalAskBarInputProps): JSX.Element {
  const initialConfig = useMemo(
    () => ({
      namespace: 'askbar',
      nodes: [CommandNode],
      onError: handleEditorError,
      theme: {},
    }),
    [],
  );

  return (
    <LexicalComposer initialConfig={initialConfig}>
      <PlainTextPlugin
        contentEditable={
          <ContentEditable
            ref={inputRef}
            data-testid="askbar-input"
            className={contentEditableClassName}
            aria-placeholder={placeholder}
            placeholder={
              <div className="askbar-placeholder thuki-text-base">
                {placeholder}
              </div>
            }
          />
        }
        ErrorBoundary={LexicalErrorBoundary}
      />
      <HistoryPlugin />
      <AutoFocusPlugin />
      {/* BehaviorPlugin registers the highlight transform before ValueSyncPlugin
          seeds any initial text, so a pre-filled value is highlighted on mount
          (newly registered transforms do not reprocess already-committed nodes). */}
      <BehaviorPlugin
        suggestionsOpen={suggestionsOpen}
        keyHandlers={keyHandlers}
        onPaste={onPaste}
      />
      <ValueSyncPlugin
        value={value}
        onValueChange={onValueChange}
        onFirstKeystroke={onFirstKeystroke}
      />
    </LexicalComposer>
  );
}
