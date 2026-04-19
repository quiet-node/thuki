import type { SearchWarning } from '../types/search';
import {
  SEARCH_WARNING_COPY,
  SEARCH_WARNING_SEVERITY,
} from '../config/searchWarnings';

interface SearchWarningIconProps {
  /**
   * Accumulated pipeline warnings for a turn. Empty array renders nothing.
   * Multiple warnings stack their copy in the tooltip and escalate to the
   * error styling if any entry is error-severity.
   */
  warnings: SearchWarning[];
}

/**
 * Subtle status icon shown beside the Sources collapsible in a search
 * answer bubble. Renders nothing when the turn has no warnings. Uses a
 * native HTML `title` tooltip so there is no extra dependency or portal
 * layer for this small affordance.
 */
export function SearchWarningIcon({ warnings }: SearchWarningIconProps) {
  if (warnings.length === 0) return null;

  const isError = warnings.some(
    (w) => SEARCH_WARNING_SEVERITY[w] === 'error',
  );
  const severity: 'warn' | 'error' = isError ? 'error' : 'warn';
  const label = isError ? 'error' : 'warning';
  const glyph = isError ? '\u2297' : '\u26A0'; // ⊗ vs ⚠
  const tooltip = warnings
    .map((w) => SEARCH_WARNING_COPY[w])
    .join('\n');

  return (
    <span
      role="img"
      aria-label={label}
      data-severity={severity}
      title={tooltip}
      className={`search-warning-icon search-warning-icon--${severity} inline-flex items-center justify-center text-xs`}
      style={{
        width: 14,
        height: 14,
        cursor: 'help',
        color: severity === 'error' ? '#b00020' : '#c68a00',
      }}
    >
      {glyph}
    </span>
  );
}
