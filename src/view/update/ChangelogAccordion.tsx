import { useCallback, useMemo, useState } from 'react';
import {
  AnimatePresence,
  motion,
  useReducedMotion,
} from 'framer-motion';

import { MarkdownRenderer } from '../../components/MarkdownRenderer';
import type { ChangelogSection } from './changelog';

/**
 * Thuki signature ease: calm decelerate used by the ask-bar morph and
 * command-suggestion expand (cubic-bezier(0.16, 1, 0.3, 1)).
 */
const THUKI_EASE = [0.16, 1, 0.3, 1] as const;

interface ChangelogAccordionProps {
  sections: ChangelogSection[];
  /**
   * When true, the newest section (first in `sections`) shows a quiet
   * "Latest" pill. Settings Changelog enables this; What's New can omit.
   */
  showLatestPill?: boolean;
}

/**
 * Collapsible per-version release notes for What's New and Settings Changelog.
 * Newest section starts expanded; older rows stay collapsed. Bodies animate
 * open/closed with the same height+opacity ease as the ask-bar popovers so
 * the motion matches the rest of Thuki. Content reuses `MarkdownRenderer`.
 */
export function ChangelogAccordion({
  sections,
  showLatestPill = false,
}: ChangelogAccordionProps) {
  const reduceMotion = useReducedMotion();
  const [expanded, setExpanded] = useState<Set<string>>(
    () => new Set(sections.slice(0, 1).map((s) => s.version)),
  );

  /**
   * Expand/collapse timing. Zeroed when the user prefers reduced motion so
   * toggles stay instantaneous without mid-state freezes.
   */
  const bodyTransition = useMemo(
    () =>
      reduceMotion
        ? { duration: 0 }
        : {
            height: { duration: 0.28, ease: THUKI_EASE },
            opacity: { duration: 0.2, ease: 'easeOut' as const },
          },
    [reduceMotion],
  );

  /**
   * Toggles a single version row between open and closed.
   */
  const toggle = useCallback((version: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(version)) next.delete(version);
      else next.add(version);
      return next;
    });
  }, []);

  return (
    <div data-testid="changelog-accordion">
      {sections.map((section, index) => {
        const isOpen = expanded.has(section.version);
        const isLatest = showLatestPill && index === 0;
        return (
          <div
            key={section.version}
            className="border-b border-white/[0.045] last:border-b-0"
          >
            <button
              type="button"
              onClick={() => toggle(section.version)}
              aria-expanded={isOpen}
              className="flex w-full items-center gap-2 py-[10px] text-left cursor-pointer"
            >
              <svg
                viewBox="0 0 16 16"
                aria-hidden="true"
                className={`h-3 w-3 shrink-0 text-text-secondary transition-transform ${
                  reduceMotion
                    ? 'duration-0'
                    : 'duration-[280ms] ease-[cubic-bezier(0.16,1,0.3,1)]'
                } ${isOpen ? 'rotate-90' : ''}`}
              >
                <path
                  d="M6 4l4 4-4 4"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.6"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
              <span className="text-[13px] font-semibold text-text-primary">
                {section.version}
              </span>
              {section.date ? (
                <span className="text-[12px] text-text-secondary">
                  {section.date}
                </span>
              ) : null}
              {isLatest ? (
                <span
                  data-testid="changelog-latest-pill"
                  className="ml-0.5 rounded-full px-1.5 py-px text-[10px] font-semibold tracking-wide text-primary bg-primary/[0.14]"
                >
                  Latest
                </span>
              ) : null}
            </button>
            {/* initial={false}: skip entrance on first paint for the
                pre-expanded newest row; still animate every user toggle. */}
            <AnimatePresence initial={false}>
              {isOpen ? (
                <motion.div
                  key={`${section.version}-body`}
                  initial={reduceMotion ? false : { height: 0, opacity: 0 }}
                  animate={{ height: 'auto', opacity: 1 }}
                  exit={reduceMotion ? undefined : { height: 0, opacity: 0 }}
                  transition={bodyTransition}
                  style={{ overflow: 'hidden' }}
                >
                  <div className="pb-3 pl-5">
                    <MarkdownRenderer content={section.body} />
                  </div>
                </motion.div>
              ) : null}
            </AnimatePresence>
          </div>
        );
      })}
    </div>
  );
}
