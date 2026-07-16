import { useCallback, useState } from 'react';

import { MarkdownRenderer } from '../../components/MarkdownRenderer';
import type { ChangelogSection } from './changelog';

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
 * Newest section starts expanded; older rows stay collapsed. Body reuses
 * `MarkdownRenderer` so links open in the system browser.
 */
export function ChangelogAccordion({
  sections,
  showLatestPill = false,
}: ChangelogAccordionProps) {
  const [expanded, setExpanded] = useState<Set<string>>(
    () => new Set(sections.slice(0, 1).map((s) => s.version)),
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
                className={`h-3 w-3 shrink-0 text-text-secondary transition-transform duration-150 ${
                  isOpen ? 'rotate-90' : ''
                }`}
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
            {isOpen ? (
              <div className="pb-3 pl-5">
                <MarkdownRenderer content={section.body} />
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
