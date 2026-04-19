/**
 * Static setup-guidance card rendered when the `/search` pre-flight probe
 * reports that the sandbox containers are not running. Not a generic error
 * bubble: styled as a warning/setup prompt with a code snippet for the
 * start command.
 */
export function SandboxSetupCard() {
  return (
    <div
      data-testid="sandbox-setup-card"
      className="flex items-stretch gap-3 px-1 py-2 rounded-md bg-white/[0.025]"
    >
      <div
        data-warning-bar
        className="w-[2.5px] rounded-sm flex-shrink-0 self-stretch min-h-[36px]"
        style={{ background: '#f59e0b' }}
      />
      <div>
        <p className="text-[12.5px] font-[590] text-white/[0.82] leading-snug tracking-[-0.01em]">
          Search sandbox not running
        </p>
        <p className="text-[11.5px] text-white/[0.38] leading-snug mt-0.5">
          Start it with{' '}
          <code className="font-mono text-[10.5px] bg-white/[0.07] text-white/50 px-[5px] py-px rounded">
            bun run search-box:start
          </code>
          , then try again.
        </p>
      </div>
    </div>
  );
}
