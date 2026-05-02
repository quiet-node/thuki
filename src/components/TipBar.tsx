import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

const NOISE_CHARS = '!@#$%^&*<>?/|abcdefghijklmnopqrstuvwxyz0123456789░▒';
const CHAR_DELAY = 36;
const FLICKER_MS = 40;
const FLICKER_COUNT = 4;
const FADE_MS = 280;

/**
 * Tips containing a bare hostname like `github.com/...` are detected and the
 * whole tip bar becomes a clickable affordance that opens the URL in the
 * user's default browser via the Tauri `open_url` command. This keeps the
 * typewriter animation intact (a real `<a>` would be torn down each pass)
 * and works for DMG users who do not have the source repo on disk.
 */
const URL_PATTERN = /(https?:\/\/\S+|(?:github|docs)\.[a-z0-9./_#?=:-]+)/i;

function extractUrl(tip: string): string | null {
  const match = tip.match(URL_PATTERN);
  if (!match) return null;
  const raw = match[0];
  return raw.startsWith('http') ? raw : `https://${raw}`;
}

interface TipBarProps {
  tip: string;
  tipKey: number;
}

export function TipBar({ tip, tipKey }: TipBarProps) {
  const url = extractUrl(tip);
  const spanRef = useRef<HTMLSpanElement>(null);
  const timersRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  useEffect(() => {
    const span = spanRef.current;
    /* v8 ignore start -- ref is always set post-mount */
    if (!span) return;
    /* v8 ignore stop */

    timersRef.current.forEach(clearTimeout);
    timersRef.current = [];

    const addTimer = (fn: () => void, ms: number) => {
      // eslint-disable-next-line @eslint-react/web-api-no-leaked-timeout
      const id = setTimeout(fn, ms);
      timersRef.current.push(id);
    };

    const runTypewriter = () => {
      const chars = tip.split('');
      span.innerHTML = chars
        .map((_, i) => `<span data-ci="${i}"></span>`)
        .join('');

      chars.forEach((ch, i) => {
        const el = span.querySelector<HTMLSpanElement>(`[data-ci="${i}"]`)!;

        if (ch === ' ') {
          addTimer(() => {
            el.textContent = ' ';
          }, i * CHAR_DELAY);
          return;
        }

        for (let f = 0; f < FLICKER_COUNT; f++) {
          addTimer(
            () => {
              /* v8 ignore next -- flicker color is visual-only */
              el.style.color = '#ff8d5c';
              el.textContent =
                NOISE_CHARS[Math.floor(Math.random() * NOISE_CHARS.length)];
            },
            i * CHAR_DELAY + f * FLICKER_MS,
          );
        }

        addTimer(
          () => {
            /* v8 ignore next -- color reset is visual-only */
            el.style.color = '#8a8a8e';
            el.textContent = ch;
          },
          i * CHAR_DELAY + FLICKER_COUNT * FLICKER_MS,
        );
      });
    };

    if (tipKey === 0) {
      runTypewriter();
    } else {
      /* v8 ignore start -- fade-out style transitions are visual-only */
      span.style.opacity = '0';
      span.style.filter = 'blur(4px)';
      span.style.transition = `opacity ${FADE_MS}ms ease, filter ${FADE_MS}ms ease`;
      /* v8 ignore stop */

      addTimer(() => {
        /* v8 ignore start -- style reset before next tip is visual-only */
        span.style.transition = '';
        span.style.opacity = '';
        span.style.filter = '';
        /* v8 ignore stop */
        runTypewriter();
      }, FADE_MS);
    }

    return () => {
      timersRef.current.forEach(clearTimeout);
      timersRef.current = [];
    };
  }, [tip, tipKey]);

  if (url) {
    return (
      <button
        type="button"
        onClick={() => {
          void invoke('open_url', { url });
        }}
        className="flex w-full items-center justify-center gap-1.5 border-t border-white/5 px-4 py-[5px] cursor-pointer transition-colors hover:bg-white/[0.03]"
        data-testid="tip-bar"
        aria-label={`Open tip link: ${url}`}
      >
        <span className="text-[9px] font-bold tracking-widest uppercase text-[#ff8d5c] bg-[#ff8d5c]/10 rounded px-1.5 py-0.5 flex-shrink-0">
          TIP
        </span>
        <span
          ref={spanRef}
          className="text-[10px]"
          style={{ color: '#8a8a8e' }}
          data-testid="tip-text"
        />
      </button>
    );
  }

  return (
    <div
      className="flex items-center justify-center gap-1.5 border-t border-white/5 px-4 py-[5px]"
      data-testid="tip-bar"
    >
      <span className="text-[9px] font-bold tracking-widest uppercase text-[#ff8d5c] bg-[#ff8d5c]/10 rounded px-1.5 py-0.5 flex-shrink-0">
        TIP
      </span>
      <span
        ref={spanRef}
        className="text-[10px]"
        style={{ color: '#8a8a8e' }}
        data-testid="tip-text"
      />
    </div>
  );
}
