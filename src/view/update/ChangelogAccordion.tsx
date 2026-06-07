import { useCallback, useState } from 'react';

import { MarkdownRenderer } from '../../components/MarkdownRenderer';
import type { ChangelogSection } from './changelog';

interface ChangelogAccordionProps {
  sections: ChangelogSection[];
}

/**
 * Collapsible per-version release notes for the "What's New" window. The newest
 * version (first, since `selectSections` sorts newest-first) starts expanded;
 * older versions collapse to a clickable header row so a multi-version jump does
 * not become a wall of text. Each body reuses `MarkdownRenderer`, so links open
 * in the system browser like everywhere else.
 */
export function ChangelogAccordion({ sections }: ChangelogAccordionProps) {
  const [expanded, setExpanded] = useState<Set<string>>(
    () => new Set(sections.slice(0, 1).map((s) => s.version)),
  );

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
      {sections.map((section) => {
        const isOpen = expanded.has(section.version);
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
              <span className="text-[13px] font-bold text-primary">
                {section.version}
              </span>
              {section.date ? (
                <span className="text-[12px] text-text-secondary">
                  {section.date}
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
