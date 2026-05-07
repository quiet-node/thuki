import { invoke } from '@tauri-apps/api/core';

interface UpdateFooterBarProps {
  version: string;
  notesUrl: string | null;
  onInstall: () => void;
  onLater: () => void;
}

export function UpdateFooterBar({
  version,
  notesUrl,
  onInstall,
  onLater,
}: UpdateFooterBarProps) {
  const openNotes = () => {
    if (notesUrl) void invoke('open_url', { url: notesUrl });
  };

  return (
    <div
      className="flex w-full items-center justify-center gap-1.5 border-t border-white/5 px-4 py-[5px]"
      data-testid="update-footer-bar"
    >
      <span className="text-[9px] font-bold tracking-widest uppercase text-[#e69c05] bg-[#e69c05]/10 rounded px-1.5 py-0.5 flex-shrink-0">
        UPD
      </span>
      <span className="text-[10px] text-[#8a8a8e]">
        <button
          type="button"
          onClick={openNotes}
          className="text-[#f0f0f2] underline decoration-dotted underline-offset-2 decoration-[#ff8d5c]/40 cursor-pointer bg-transparent border-0 p-0 font-inherit"
        >
          {`v${version}`}
        </button>
        {' ready · '}
        <button
          type="button"
          onClick={onInstall}
          className="text-[#ff8d5c] underline decoration-dotted underline-offset-2 decoration-[#ff8d5c]/50 cursor-pointer bg-transparent border-0 p-0 font-inherit"
        >
          install &amp; restart
        </button>
        {' · '}
        <button
          type="button"
          onClick={onLater}
          className="text-[#8a8a8e] underline decoration-dotted underline-offset-2 decoration-[#8a8a8e]/50 cursor-pointer bg-transparent border-0 p-0 font-inherit"
        >
          later
        </button>
      </span>
    </div>
  );
}
