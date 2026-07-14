/**
 * Renders a short markdown-ish attribution line with clickable links.
 *
 * Only supports the subset we emit from the backend: plain text mixed with
 * `[label](https://…)` links. Opens links via the Tauri `open_url` command.
 */

import { invoke } from '@tauri-apps/api/core';
import { Fragment, type ReactNode } from 'react';

const LINK_RE = /\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g;

/**
 * Splits `md` into text and link segments and returns React nodes.
 * Unknown markdown is left as plain text.
 */
export function renderAttributionMarkdown(md: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let last = 0;
  let match: RegExpExecArray | null;
  const re = new RegExp(LINK_RE.source, 'g');
  while ((match = re.exec(md)) !== null) {
    if (match.index > last) {
      const text = md.slice(last, match.index);
      nodes.push(<Fragment key={`t-${match.index}-${text}`}>{text}</Fragment>);
    }
    const label = match[1];
    const href = match[2];
    nodes.push(
      <button
        key={`a-${match.index}-${href}`}
        type="button"
        className="underline decoration-white/25 hover:decoration-white/50 text-inherit"
        onClick={(e) => {
          e.stopPropagation();
          void invoke('open_url', { url: href });
        }}
      >
        {label}
      </button>,
    );
    last = match.index + match[0].length;
  }
  if (last < md.length) {
    const tail = md.slice(last);
    nodes.push(<Fragment key={`t-end-${tail}`}>{tail}</Fragment>);
  }
  return nodes;
}

interface SourceAttributionProps {
  /** Markdown attribution string from `SearchResultPreview.attribution`. */
  markdown: string;
}

/**
 * Compact attribution line under a source row (licence / provider credit).
 */
export function SourceAttribution({ markdown }: SourceAttributionProps) {
  return (
    <p
      data-testid="source-attribution"
      className="pl-8 text-[11px] text-white/30 leading-snug"
    >
      {renderAttributionMarkdown(markdown)}
    </p>
  );
}
