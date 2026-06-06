import { useCallback, useEffect, useRef, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Tooltip } from './Tooltip';

interface ReplaceButtonProps {
  /** Rewritten text to write back into the source app. */
  content: string;
  /**
   * Writes `content` into the source app, replacing the user's selection. The
   * paste lands in the source app while the overlay stays open, so the user
   * can replace repeatedly. Resolves to whether the write succeeded, so the
   * button can confirm a successful replace with a tick.
   */
  onReplace: (text: string) => Promise<boolean>;
}

/**
 * Icon-only button rendered below a `/rewrite` or `/refine` result. Writes the
 * rewritten text back into the source app, replacing the user's selection. On a
 * successful write it flips to a checkmark for 1.5s, then reverts: the paste
 * lands in another app where the confirmation is not otherwise visible, so the
 * tick is the only signal it worked (mirrors the sibling `CopyButton`). A
 * skipped write (no target / secure field) leaves the button unchanged. A hover
 * tooltip (the same `Tooltip` the chat header icons use) names the action.
 */
export function ReplaceButton({ content, onReplace }: ReplaceButtonProps) {
  const [replaced, setReplaced] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const handleReplace = useCallback(async () => {
    const ok = await onReplace(content);
    if (!ok) return;
    if (timerRef.current) clearTimeout(timerRef.current);
    setReplaced(true);
    timerRef.current = setTimeout(() => setReplaced(false), 1500);
  }, [content, onReplace]);

  useEffect(
    () => () => {
      if (timerRef.current) clearTimeout(timerRef.current);
    },
    [],
  );

  return (
    <Tooltip label="Replace selection">
      <button
        onClick={handleReplace}
        className="transition-opacity duration-150 text-white/40 hover:text-white/70 p-0.5 rounded cursor-pointer shrink-0 flex"
        aria-label={replaced ? 'Replaced' : 'Replace selection in source app'}
      >
        <AnimatePresence mode="wait" initial={false}>
          {replaced ? (
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
              key="replace"
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
                <rect x="3" y="4" width="12" height="16" rx="2" />
                <line x1="21" y1="12" x2="9" y2="12" />
                <polyline points="13 8 9 12 13 16" />
              </svg>
            </motion.span>
          )}
        </AnimatePresence>
      </button>
    </Tooltip>
  );
}
