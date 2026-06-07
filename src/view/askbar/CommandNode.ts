import type { EditorConfig, LexicalNode, SerializedTextNode } from 'lexical';
import { $applyNodeReplacement, TextNode } from 'lexical';
import { addClassNamesToElement } from '@lexical/utils';

/**
 * A TextNode subclass that renders a recognized slash-command trigger (e.g.
 * "/search") in violet. `registerLexicalTextEntity` swaps matched text into
 * this node as the user types and back to plain text when the token stops
 * matching, so the highlight tracks the caret natively with no overlay layer.
 *
 * Mirrors the canonical `@lexical/hashtag` HashtagNode pattern.
 */
export class CommandNode extends TextNode {
  static getType(): string {
    return 'command';
  }

  static clone(node: CommandNode): CommandNode {
    return new CommandNode(node.__text, node.__key);
  }

  static importJSON(serializedNode: SerializedTextNode): CommandNode {
    return $createCommandNode().updateFromJSON(serializedNode);
  }

  createDOM(config: EditorConfig): HTMLElement {
    const element = super.createDOM(config);
    addClassNamesToElement(element, 'text-violet-400');
    return element;
  }

  /** Marks this node as a text entity so the entity transform tracks boundaries. */
  isTextEntity(): true {
    return true;
  }

  /** Typing immediately before the token must not extend it. */
  canInsertTextBefore(): boolean {
    return false;
  }
}

/** Creates a CommandNode wrapping `text`, applying any registered node replacement. */
export function $createCommandNode(text = ''): CommandNode {
  return $applyNodeReplacement(new CommandNode(text));
}

/** Type guard for CommandNode. */
export function $isCommandNode(
  node: LexicalNode | null | undefined,
): node is CommandNode {
  return node instanceof CommandNode;
}
