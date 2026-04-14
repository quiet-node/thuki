import { motion } from 'framer-motion';
import { MarkdownRenderer } from './MarkdownRenderer';
import { ErrorCard } from './ErrorCard';
import { CopyButton } from './CopyButton';
import { ImageThumbnails } from './ImageThumbnails';
import { ThinkingBlock } from './ThinkingBlock';
import { convertFileSrc } from '@tauri-apps/api/core';
import { formatQuotedText } from '../utils/formatQuote';
import { quote } from '../config';
import { COMMANDS, SCREEN_CAPTURE_PLACEHOLDER } from '../config/commands';
import type { OllamaErrorKind } from '../hooks/useOllama';

/**
 * Renders user message content with slash commands styled distinctly.
 * Finds ALL command triggers anywhere in the text and wraps each in a
 * styled span so they stand out in the orange user bubble.
 */
function renderUserContent(content: string): React.ReactNode {
  const parts: React.ReactNode[] = [];
  let remaining = content;

  while (remaining.length > 0) {
    // Find the earliest command trigger in remaining text
    let earliest = -1;
    let matchedTrigger = '';
    for (const cmd of COMMANDS) {
      const idx = remaining.indexOf(cmd.trigger);
      if (idx !== -1 && (earliest === -1 || idx < earliest)) {
        // Verify it's a whole word (preceded by start/space, followed by end/space)
        const before = idx === 0 || remaining[idx - 1] === ' ';
        const after =
          idx + cmd.trigger.length >= remaining.length ||
          remaining[idx + cmd.trigger.length] === ' ';
        if (before && after) {
          earliest = idx;
          matchedTrigger = cmd.trigger;
        }
      }
    }

    if (earliest === -1) {
      parts.push(<span key={parts.length}>{remaining}</span>);
      break;
    }

    // Text before the command
    if (earliest > 0) {
      parts.push(
        <span key={parts.length}>{remaining.slice(0, earliest)}</span>,
      );
    }
    // The command itself, styled
    parts.push(
      <span key={parts.length} className="font-semibold text-[#7C2D12]">
        {matchedTrigger}
      </span>,
    );
    remaining = remaining.slice(earliest + matchedTrigger.length);
  }

  return <>{parts}</>;
}

interface ChatBubbleProps {
  /** The message role determines alignment and color treatment. */
  role: 'user' | 'assistant';
  /** The message content to render. AI messages support markdown. */
  content: string;
  /** Stagger index for orchestrated entrance choreography. */
  index: number;
  /** Selected text from the host app that was quoted alongside this message, if any. */
  quotedText?: string;
  /** Whether this bubble is actively streaming content from the LLM. */
  isStreaming?: boolean;
  /** When set, renders an ErrorCard callout instead of markdown. */
  errorKind?: OllamaErrorKind;
  /** Accumulated thinking/reasoning content from the model, if thinking mode was used. */
  thinkingContent?: string;
  /** Duration of the thinking phase in milliseconds. */
  thinkingDurationMs?: number;
  /** Whether the model is currently in the thinking phase (streaming thinking tokens). */
  isThinking?: boolean;
  /** Absolute file paths of images attached to this message, if any. */
  imagePaths?: string[];
  /** Called when the user clicks a thumbnail to preview it. */
  onImagePreview?: (path: string) => void;
}

/**
 * Framer Motion variants for individual chat bubbles.
 * Uses GPU-accelerated transforms (opacity, y, scale) for jank-free animation.
 * Spring physics provide natural, organic motion.
 */
const bubbleVariants = {
  hidden: { opacity: 0, y: 12, scale: 0.95 },
  visible: {
    opacity: 1,
    y: 0,
    scale: 1,
    transition: {
      type: 'spring' as const,
      stiffness: 380,
      damping: 26,
    },
  },
};

/**
 * Renders a chat message following industry-standard assistant UI conventions:
 *
 * - **User messages** — right-aligned bubble with warm gradient, quoted-text
 *   support, and an always-visible copy button below the bubble (right-aligned).
 * - **AI messages** — full-width plain text (no bubble), markdown-rendered, with
 *   an always-visible copy button below the text (left-aligned).
 *
 * Spring entrance animation is staggered by `index` to produce natural
 * choreography when multiple messages appear at once.
 */
export function ChatBubble({
  role,
  content,
  index,
  quotedText,
  isStreaming = false,
  imagePaths,
  onImagePreview,
  errorKind,
  thinkingContent,
  thinkingDurationMs,
  isThinking,
}: ChatBubbleProps) {
  const isUser = role === 'user';

  return (
    <motion.div
      variants={bubbleVariants}
      initial="hidden"
      animate="visible"
      transition={{ delay: index * 0.06 }}
      className={`flex w-full ${isUser ? 'justify-end' : 'justify-start'}`}
    >
      {isUser ? (
        /* User bubble — max-width capped, stacks bubble + action bar */
        <div className="flex flex-col max-w-[80%]">
          <div className="chat-bubble chat-bubble-user relative px-4 py-2.5 text-sm leading-relaxed select-text rounded-2xl rounded-br-md">
            {quotedText && (
              <p className="border-l-2 border-white/40 pl-2 mb-2 italic text-xs text-white/60 whitespace-pre-wrap">
                {formatQuotedText(
                  quotedText,
                  quote.maxDisplayLines,
                  quote.maxDisplayChars,
                )}
              </p>
            )}
            {imagePaths && imagePaths.length > 0 && onImagePreview && (
              <div className="mb-2">
                <ImageThumbnails
                  items={imagePaths.map((p) => ({
                    id: p,
                    src:
                      p === SCREEN_CAPTURE_PLACEHOLDER
                        ? p
                        : p.startsWith('blob:')
                          ? p
                          : convertFileSrc(p),
                    loading: p.startsWith('blob:'),
                    placeholder: p === SCREEN_CAPTURE_PLACEHOLDER,
                  }))}
                  onPreview={onImagePreview}
                  size={48}
                />
              </div>
            )}
            {content && (
              <span className="text-white/95 font-medium whitespace-pre-wrap">
                {renderUserContent(content)}
              </span>
            )}
          </div>
          {content && (
            <div className="h-6 flex items-center px-1">
              <CopyButton content={content} align="right" />
            </div>
          )}
        </div>
      ) : (
        /* AI plain text — full width, no bubble chrome */
        <div className="flex flex-col w-full">
          <div className="text-sm leading-relaxed select-text py-1">
            {thinkingContent && (
              <ThinkingBlock
                thinkingContent={thinkingContent}
                isThinking={isThinking ?? false}
                durationMs={thinkingDurationMs}
              />
            )}
            {errorKind ? (
              <ErrorCard kind={errorKind} message={content} />
            ) : (
              <MarkdownRenderer content={content} isStreaming={isStreaming} />
            )}
          </div>
          {!errorKind && !isStreaming && (
            <div className="h-6 flex items-center">
              <CopyButton content={content} align="left" />
            </div>
          )}
        </div>
      )}
    </motion.div>
  );
}
