import { invoke } from '@tauri-apps/api/core';

/**
 * Static setup-guidance card rendered when the `/search` pre-flight probe
 * reports that the sandbox containers are not running. Not a generic error
 * bubble: styled as a warning/setup prompt with a code snippet for the
 * start command.
 */
export function SandboxSetupCard() {
  const SETUP_URL =
    'https://github.com/quiet-node/thuki#step-2-setup-the-search-sandbox-optional-required-for-search';

  const handleOpenGuide = () => {
    void invoke('open_url', { url: SETUP_URL });
  };

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
          Search service is offline
        </p>
        <p className="text-[11.5px] text-white/[0.38] leading-snug mt-0.5">
          Follow the{' '}
          <button
            type="button"
            onClick={handleOpenGuide}
            className="text-white/50 underline decoration-white/20 underline-offset-2 hover:text-white/70 transition-colors cursor-pointer"
          >
            Setup Guide
          </button>{' '}
          to enable local agentic search.
        </p>
      </div>
    </div>
  );
}
