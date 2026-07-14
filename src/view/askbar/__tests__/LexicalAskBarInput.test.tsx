import { describe, it, expect, vi } from 'vitest';
import type { ReactNode } from 'react';
import { useEffect } from 'react';
import { render, screen, act, waitFor } from '@testing-library/react';
import { LexicalComposer } from '@lexical/react/LexicalComposer';
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext';
import { PlainTextPlugin } from '@lexical/react/LexicalPlainTextPlugin';
import { ContentEditable } from '@lexical/react/LexicalContentEditable';
import { LexicalErrorBoundary } from '@lexical/react/LexicalErrorBoundary';
import {
  $createParagraphNode,
  $createTextNode,
  $getRoot,
  $nodesOfType,
  KEY_ARROW_DOWN_COMMAND,
  KEY_ARROW_UP_COMMAND,
  KEY_ENTER_COMMAND,
  KEY_ESCAPE_COMMAND,
  KEY_TAB_COMMAND,
  PASTE_COMMAND,
} from 'lexical';
import type { LexicalEditor } from 'lexical';
import {
  LexicalAskBarInput,
  ValueSyncPlugin,
  BehaviorPlugin,
  handleEditorError,
} from '../LexicalAskBarInput';
import type { AskBarKeyHandlers } from '../LexicalAskBarInput';
import { CommandNode } from '../CommandNode';

const HARNESS_CONFIG = {
  namespace: 'askbar-test',
  nodes: [CommandNode],
  onError: (error: Error) => {
    throw error;
  },
};

/** Headless composer harness that exposes the editor instance to the test. */
function Composer({
  children,
  onEditor,
}: {
  children: ReactNode;
  onEditor?: (editor: LexicalEditor) => void;
}) {
  return (
    <LexicalComposer initialConfig={HARNESS_CONFIG}>
      <PlainTextPlugin
        contentEditable={<ContentEditable data-testid="ce" />}
        ErrorBoundary={LexicalErrorBoundary}
      />
      {onEditor && <Capture onEditor={onEditor} />}
      {children}
    </LexicalComposer>
  );
}

function Capture({ onEditor }: { onEditor: (editor: LexicalEditor) => void }) {
  const [editor] = useLexicalComposerContext();
  useEffect(() => {
    onEditor(editor);
  }, [editor, onEditor]);
  return null;
}

/** Seeds the editor with a single paragraph holding `text`. */
function seed(editor: LexicalEditor, text: string) {
  act(() => {
    editor.update(() => {
      const root = $getRoot();
      root.clear();
      const paragraph = $createParagraphNode();
      paragraph.append($createTextNode(text));
      root.append(paragraph);
    });
  });
}

function makeKeyHandlers(): AskBarKeyHandlers {
  return {
    onEnterKey: vi.fn(),
    onArrowDown: vi.fn(),
    onArrowUp: vi.fn(),
    onTab: vi.fn(),
    onEscape: vi.fn(),
  };
}

describe('handleEditorError', () => {
  it('rethrows the error', () => {
    expect(() => handleEditorError(new Error('boom'))).toThrow('boom');
  });
});

describe('ValueSyncPlugin', () => {
  it('pushes editor edits up and fires first keystroke on empty→non-empty', async () => {
    let editor: LexicalEditor | null = null;
    const onValueChange = vi.fn();
    const onFirstKeystroke = vi.fn();
    render(
      <Composer onEditor={(e) => (editor = e)}>
        <ValueSyncPlugin
          value=""
          onValueChange={onValueChange}
          onFirstKeystroke={onFirstKeystroke}
        />
      </Composer>,
    );
    seed(editor!, 'hello');
    await waitFor(() =>
      expect(onValueChange).toHaveBeenLastCalledWith('hello'),
    );
    expect(onFirstKeystroke).toHaveBeenCalledTimes(1);
  });

  it('does not fire first keystroke when the value was already non-empty', async () => {
    let editor: LexicalEditor | null = null;
    const onValueChange = vi.fn();
    const onFirstKeystroke = vi.fn();
    render(
      <Composer onEditor={(e) => (editor = e)}>
        <ValueSyncPlugin
          value="a"
          onValueChange={onValueChange}
          onFirstKeystroke={onFirstKeystroke}
        />
      </Composer>,
    );
    // value="a" seeds the editor via the down-sync; that echo must not call back.
    await waitFor(() =>
      expect(
        editor!.getEditorState().read(() => $getRoot().getTextContent()),
      ).toBe('a'),
    );
    onValueChange.mockClear();
    seed(editor!, 'ab');
    await waitFor(() => expect(onValueChange).toHaveBeenLastCalledWith('ab'));
    expect(onFirstKeystroke).not.toHaveBeenCalled();
  });

  it('tolerates a missing onFirstKeystroke callback', async () => {
    let editor: LexicalEditor | null = null;
    const onValueChange = vi.fn();
    render(
      <Composer onEditor={(e) => (editor = e)}>
        <ValueSyncPlugin value="" onValueChange={onValueChange} />
      </Composer>,
    );
    seed(editor!, 'hi');
    await waitFor(() => expect(onValueChange).toHaveBeenLastCalledWith('hi'));
  });

  it('does not echo when the editor text already equals the value', async () => {
    let editor: LexicalEditor | null = null;
    const onValueChange = vi.fn();
    render(
      <Composer onEditor={(e) => (editor = e)}>
        <ValueSyncPlugin value="seed" onValueChange={onValueChange} />
      </Composer>,
    );
    await waitFor(() =>
      expect(
        editor!.getEditorState().read(() => $getRoot().getTextContent()),
      ).toBe('seed'),
    );
    // The down-sync set the text; the resulting update echoes equal → no call.
    expect(onValueChange).not.toHaveBeenCalled();
  });

  it('rebuilds the editor from an external multi-line value', async () => {
    let editor: LexicalEditor | null = null;
    const onValueChange = vi.fn();
    const { rerender } = render(
      <Composer onEditor={(e) => (editor = e)}>
        <ValueSyncPlugin value="" onValueChange={onValueChange} />
      </Composer>,
    );
    rerender(
      <Composer onEditor={(e) => (editor = e)}>
        <ValueSyncPlugin value={'a\n\nb'} onValueChange={onValueChange} />
      </Composer>,
    );
    await waitFor(() =>
      expect(
        editor!.getEditorState().read(() => $getRoot().getTextContent()),
      ).toBe('a\n\nb'),
    );
  });
});

describe('BehaviorPlugin', () => {
  function renderBehavior(
    overrides: Partial<{
      suggestionsOpen: boolean;
      keyHandlers: AskBarKeyHandlers;
      onPaste: (clipboard: DataTransfer | null) => boolean;
    }> = {},
  ) {
    const keyHandlers = overrides.keyHandlers ?? makeKeyHandlers();
    const onPaste = overrides.onPaste ?? vi.fn(() => false);
    let editor: LexicalEditor | null = null;
    const ui = (props: typeof overrides) => (
      <Composer onEditor={(e) => (editor = e)}>
        <BehaviorPlugin
          suggestionsOpen={props.suggestionsOpen ?? false}
          keyHandlers={props.keyHandlers ?? keyHandlers}
          onPaste={props.onPaste ?? onPaste}
        />
      </Composer>
    );
    const utils = render(ui(overrides));
    return {
      getEditor: () => editor as LexicalEditor,
      keyHandlers,
      onPaste,
      rerender: (props: typeof overrides) => utils.rerender(ui(props)),
    };
  }

  it('highlights a recognized command token via a CommandNode', async () => {
    const h = renderBehavior();
    seed(h.getEditor(), '/search foo');
    await waitFor(() =>
      expect(
        h
          .getEditor()
          .getEditorState()
          .read(() => $nodesOfType(CommandNode).length),
      ).toBe(1),
    );
  });

  it('Enter without Shift triggers the host enter handler', () => {
    const h = renderBehavior();
    let handled = false;
    act(() => {
      handled = h
        .getEditor()
        .dispatchCommand(
          KEY_ENTER_COMMAND,
          new KeyboardEvent('keydown', { shiftKey: false }),
        );
    });
    expect(handled).toBe(true);
    expect(h.keyHandlers.onEnterKey).toHaveBeenCalledTimes(1);
  });

  it('Enter with Shift falls through to the default line break', () => {
    const h = renderBehavior();
    let handled = true;
    act(() => {
      handled = h
        .getEditor()
        .dispatchCommand(
          KEY_ENTER_COMMAND,
          new KeyboardEvent('keydown', { shiftKey: true }),
        );
    });
    expect(handled).toBe(false);
    expect(h.keyHandlers.onEnterKey).not.toHaveBeenCalled();
  });

  it('Enter with a null event still submits', () => {
    const h = renderBehavior();
    act(() => {
      h.getEditor().dispatchCommand(KEY_ENTER_COMMAND, null);
    });
    expect(h.keyHandlers.onEnterKey).toHaveBeenCalledTimes(1);
  });

  it('intercepts Arrow/Tab/Escape only while the popover is open', () => {
    const h = renderBehavior({ suggestionsOpen: true });
    act(() => {
      h.getEditor().dispatchCommand(
        KEY_ARROW_DOWN_COMMAND,
        new KeyboardEvent('keydown'),
      );
      h.getEditor().dispatchCommand(
        KEY_ARROW_UP_COMMAND,
        new KeyboardEvent('keydown'),
      );
      h.getEditor().dispatchCommand(
        KEY_TAB_COMMAND,
        new KeyboardEvent('keydown'),
      );
      h.getEditor().dispatchCommand(
        KEY_ESCAPE_COMMAND,
        new KeyboardEvent('keydown'),
      );
    });
    expect(h.keyHandlers.onArrowDown).toHaveBeenCalledTimes(1);
    expect(h.keyHandlers.onArrowUp).toHaveBeenCalledTimes(1);
    expect(h.keyHandlers.onTab).toHaveBeenCalledTimes(1);
    expect(h.keyHandlers.onEscape).toHaveBeenCalledTimes(1);
  });

  it('ignores Arrow/Tab/Escape while the popover is closed', () => {
    const h = renderBehavior({ suggestionsOpen: false });
    const results: boolean[] = [];
    act(() => {
      results.push(
        h
          .getEditor()
          .dispatchCommand(
            KEY_ARROW_DOWN_COMMAND,
            new KeyboardEvent('keydown'),
          ),
        h
          .getEditor()
          .dispatchCommand(KEY_ARROW_UP_COMMAND, new KeyboardEvent('keydown')),
        h
          .getEditor()
          .dispatchCommand(KEY_TAB_COMMAND, new KeyboardEvent('keydown')),
        h
          .getEditor()
          .dispatchCommand(KEY_ESCAPE_COMMAND, new KeyboardEvent('keydown')),
      );
    });
    expect(results.every((r) => r === false)).toBe(true);
    expect(h.keyHandlers.onArrowDown).not.toHaveBeenCalled();
    expect(h.keyHandlers.onArrowUp).not.toHaveBeenCalled();
    expect(h.keyHandlers.onTab).not.toHaveBeenCalled();
    expect(h.keyHandlers.onEscape).not.toHaveBeenCalled();
  });

  it('consumes a paste the host handled, preventing the default', () => {
    const onPaste = vi.fn(() => true);
    const h = renderBehavior({ onPaste });
    const clipboardData = { items: [] } as unknown as DataTransfer;
    const preventDefault = vi.fn();
    let handled = false;
    act(() => {
      handled = h.getEditor().dispatchCommand(PASTE_COMMAND, {
        clipboardData,
        preventDefault,
      } as unknown as ClipboardEvent);
    });
    expect(onPaste).toHaveBeenCalledWith(clipboardData);
    expect(preventDefault).toHaveBeenCalledTimes(1);
    expect(handled).toBe(true);
  });

  it('lets an unhandled paste fall through with a null clipboard', () => {
    const onPaste = vi.fn(() => false);
    const h = renderBehavior({ onPaste });
    let handled = true;
    act(() => {
      handled = h.getEditor().dispatchCommand(PASTE_COMMAND, {
        preventDefault: vi.fn(),
      } as unknown as ClipboardEvent);
    });
    expect(onPaste).toHaveBeenCalledWith(null);
    expect(handled).toBe(false);
  });
});

describe('LexicalAskBarInput (integration)', () => {
  function renderInput(
    overrides: Partial<{
      value: string;
      suggestionsOpen: boolean;
      onValueChange: (v: string) => void;
      onPaste: (clipboard: DataTransfer | null) => boolean;
    }> = {},
  ) {
    const inputRef = {
      current: null,
    } as React.RefObject<HTMLDivElement | null>;
    render(
      <LexicalAskBarInput
        value={overrides.value ?? ''}
        onValueChange={overrides.onValueChange ?? vi.fn()}
        placeholder="Ask Thuki anything..."
        inputRef={inputRef}
        contentEditableClassName="askbar-input"
        suggestionsOpen={overrides.suggestionsOpen ?? false}
        keyHandlers={makeKeyHandlers()}
        onPaste={overrides.onPaste ?? vi.fn(() => false)}
      />,
    );
    return inputRef;
  }

  it('renders an editable textbox and the placeholder', () => {
    renderInput();
    expect(screen.getByTestId('askbar-input')).toBeInTheDocument();
    expect(screen.getByRole('textbox')).toBeInTheDocument();
    expect(screen.getByText('Ask Thuki anything...')).toBeInTheDocument();
  });

  it('exposes an accessible name independent of the placeholder', () => {
    renderInput();
    expect(screen.getByRole('textbox', { name: 'Ask Thuki' })).toBe(
      screen.getByTestId('askbar-input'),
    );
  });

  it('captures the contentEditable element into the inputRef', () => {
    const ref = renderInput();
    expect(ref.current).not.toBeNull();
    expect(ref.current?.getAttribute('contenteditable')).toBe('true');
  });

  it('seeds the editor from value and highlights command tokens', async () => {
    renderInput({ value: 'hi /search' });
    const input = screen.getByTestId('askbar-input');
    await waitFor(() => expect(input.textContent).toContain('/search'));
    await waitFor(() =>
      expect(input.querySelector('.text-violet-400')?.textContent).toBe(
        '/search',
      ),
    );
  });
});
