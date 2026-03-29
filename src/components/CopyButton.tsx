import { useState, useRef, useCallback, useEffect } from 'react';
import { AnimatePresence, motion } from 'framer-motion';

interface CopyButtonProps {
  /** Raw text content to write to the clipboard. */
  content: string;
  /** Which side of the action bar the button sits on — matches bubble tail side. */
  align: 'left' | 'right';
}

/**
 * One-click copy button that lives in the reserved action bar below a chat bubble.
 * Visible only when the parent container is hovered (via Tailwind `group`).
 * Shows a checkmark for 1.5s on successful copy, then reverts to the copy icon.
 * Clipboard failures are swallowed silently.
 */
export function CopyButton({ content, align }: CopyButtonProps) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(content);
      if (timerRef.current) clearTimeout(timerRef.current);
      setCopied(true);
      timerRef.current = setTimeout(() => setCopied(false), 1500);
    } catch {
      // fail silently — no clipboard access is not an error worth surfacing
    }
  }, [content]);

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    [],
  );

  return (
    <div
      className={`flex w-full ${align === 'right' ? 'justify-end' : 'justify-start'}`}
    >
      <button
        onClick={handleCopy}
        className={`transition-opacity duration-150 text-white/40 hover:text-white/70 p-0.5 rounded cursor-pointer ${copied ? 'opacity-100' : 'opacity-0 group-hover:opacity-100'}`}
        aria-label={copied ? 'Copied' : 'Copy message'}
      >
        <AnimatePresence mode="wait" initial={false}>
          {copied ? (
            <motion.span
              key="check"
              initial={{ opacity: 0, scale: 0.8 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.8 }}
              transition={{ duration: 0.1 }}
              className="flex"
            >
              <svg
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2.5"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <polyline points="20 6 9 17 4 12" />
              </svg>
            </motion.span>
          ) : (
            <motion.span
              key="copy"
              initial={{ opacity: 0, scale: 0.8 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.8 }}
              transition={{ duration: 0.1 }}
              className="flex"
            >
              <svg
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
              </svg>
            </motion.span>
          )}
        </AnimatePresence>
      </button>
    </div>
  );
}
