export type OllamaErrorKind = 'NotRunning' | 'ModelNotFound' | 'Other';

interface ErrorCardProps {
  kind: OllamaErrorKind;
  message: string;
}

const barColors: Record<OllamaErrorKind, string> = {
  NotRunning: '#ef4444',
  ModelNotFound: '#f59e0b',
  Other: 'rgba(255,255,255,0.2)',
};

/**
 * Renders a Minimal Line error callout inline in the chat thread.
 * The message is split on the first newline into title and subtitle.
 * The subtitle renders an ollama pull command as an inline code element.
 */
export function ErrorCard({ kind, message }: ErrorCardProps) {
  const newlineIndex = message.indexOf('\n');
  const title = newlineIndex === -1 ? message : message.slice(0, newlineIndex);
  const subtitle = newlineIndex === -1 ? null : message.slice(newlineIndex + 1);

  const subtitleParts = subtitle ? renderSubtitle(subtitle) : null;

  return (
    <div className="flex items-stretch gap-3 px-1 py-2 rounded-md bg-white/[0.025]">
      <div
        data-error-bar
        data-kind={kind}
        className="w-[2.5px] rounded-sm flex-shrink-0 self-stretch min-h-[36px]"
        style={{ background: barColors[kind] }}
      />
      <div>
        <p className="text-[12.5px] font-[590] text-white/[0.82] leading-snug tracking-[-0.01em]">
          {title}
        </p>
        {subtitleParts && (
          <p className="text-[11.5px] text-white/[0.38] leading-snug mt-0.5">
            {subtitleParts}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * Renders the subtitle string, wrapping `ollama pull <model>` in a <code> element.
 */
function renderSubtitle(text: string): React.ReactNode {
  const match = text.match(/^(.*?)(ollama pull \S+)(.*)$/);
  if (!match) return text;
  const [, before, command, after] = match;
  return (
    <>
      {before}
      <code className="font-mono text-[10.5px] bg-white/[0.07] text-white/50 px-[5px] py-px rounded">
        {command}
      </code>
      {after}
    </>
  );
}
