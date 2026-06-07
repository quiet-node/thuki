import { describe, it, expect } from 'vitest';
import { createEditor, TextNode } from 'lexical';
import type { EditorConfig } from 'lexical';
import {
  CommandNode,
  $createCommandNode,
  $isCommandNode,
} from '../CommandNode';

/** Minimal headless editor that knows about CommandNode, for $-prefixed APIs. */
function makeEditor() {
  return createEditor({
    namespace: 'command-node-test',
    nodes: [CommandNode],
    onError: (e) => {
      throw e;
    },
  });
}

describe('CommandNode', () => {
  it('reports its node type', () => {
    expect(CommandNode.getType()).toBe('command');
  });

  it('clones preserving text and key', () => {
    const editor = makeEditor();
    editor.update(() => {
      const original = $createCommandNode('/search');
      const cloned = CommandNode.clone(original);
      expect(cloned).toBeInstanceOf(CommandNode);
      expect(cloned.getTextContent()).toBe('/search');
      expect(cloned.__key).toBe(original.__key);
    });
  });

  it('is a text entity and blocks text insertion before it', () => {
    const editor = makeEditor();
    editor.update(() => {
      const node = $createCommandNode('/search');
      expect(node.isTextEntity()).toBe(true);
      expect(node.canInsertTextBefore()).toBe(false);
    });
  });

  it('renders a DOM element carrying the violet token class', () => {
    const editor = makeEditor();
    let element: HTMLElement | null = null;
    editor.update(() => {
      const node = $createCommandNode('/search');
      element = node.createDOM({ namespace: 'x', theme: {} } as EditorConfig);
    });
    expect(element).not.toBeNull();
    expect((element as unknown as HTMLElement).className).toContain(
      'text-violet-400',
    );
    expect((element as unknown as HTMLElement).textContent).toBe('/search');
  });

  it('round-trips through JSON serialization', () => {
    const editor = makeEditor();
    editor.update(() => {
      const node = $createCommandNode('/think');
      const json = node.exportJSON();
      const restored = CommandNode.importJSON(json);
      expect($isCommandNode(restored)).toBe(true);
      expect(restored.getTextContent()).toBe('/think');
    });
  });

  it('$isCommandNode distinguishes CommandNode from a plain TextNode', () => {
    expect($isCommandNode(null)).toBe(false);
    const editor = makeEditor();
    editor.update(() => {
      expect($isCommandNode($createCommandNode('/search'))).toBe(true);
      expect($isCommandNode(new TextNode('/search'))).toBe(false);
    });
  });
});
