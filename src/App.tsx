import { motion, AnimatePresence } from 'framer-motion';
import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useOllama } from './hooks/useOllama';
import { MarkdownRenderer } from './components/MarkdownRenderer';
import { getCurrentWindow } from '@tauri-apps/api/window';
import './App.css';

const OVERLAY_VISIBILITY_EVENT = 'thuki://visibility';

/**
 * Authoritative deadline from the start of the hide transition to the native
 * window hide call. Accounts for WKWebView `requestAnimationFrame` throttling
 * in non-key windows, which stalls spring animations indefinitely and makes
 * `AnimatePresence.onExitComplete` unreliable when the panel is unfocused.
 */
const HIDE_COMMIT_DELAY_MS = 350;

type OverlayVisibilityPayload = 'show' | 'hide-request';
type OverlayState = 'visible' | 'hidden' | 'hiding';

/**
 * Hoisted static SVG — prevents re-allocation on every render cycle.
 * @see Vercel React Best Practices §6.3 — Hoist Static JSX Elements
 */
const ARROW_UP_ICON = (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <path
      d="M8 13V3M8 3L3 8M8 3L13 8"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

/**
 * Animated spinner rendered in the submit button during response generation.
 * Defined as a component to guarantee fresh animation state on each mount.
 */
function Spinner() {
  return (
    <motion.div
      animate={{ rotate: 360 }}
      transition={{ duration: 0.7, repeat: Infinity, ease: 'linear' }}
      className="w-4 h-4 rounded-full border-2 border-neutral border-t-primary"
    />
  );
}

/**
 * Main application component for Thuki.
 *
 * Renders a minimal, spotlight-style input bar with an expanding response panel.
 * Designed as a frameless, transparent overlay for fast assistant interactions.
 */
function App() {
  const [query, setQuery] = useState('');
  const [overlayState, setOverlayState] = useState<OverlayState>('hidden');
  const { messages, streamingContent, ask, isGenerating, error, reset } =
    useOllama();
  const responseRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const canSubmit = query.trim().length > 0 && !isGenerating;

  /** Derive latest assistant response without ES2023+ Array methods. */
  const latestResponse = messages.reduce<string>(
    (acc, m) => (m.role === 'assistant' ? m.content : acc),
    '',
  );
  const displayContent = streamingContent || latestResponse;
  const showResponse =
    displayContent.length > 0 || isGenerating || error !== null;
  const shouldRenderOverlay = overlayState === 'visible';

  /**
   * Replays the entrance sequence by transitioning the overlay to the visible state.
   * Clears conversation state for a fresh session each time the overlay appears.
   */
  const replayEntranceAnimation = useCallback(() => {
    setQuery('');
    reset();
    setOverlayState('visible');
  }, [reset]);

  /**
   * Moves the overlay into an exit phase. The actual Tauri window hide call is
   * deferred until Framer Motion finishes the exit transition.
   */
  const requestHideOverlay = useCallback(() => {
    setOverlayState((currentState) => {
      if (currentState === 'hidden' || currentState === 'hiding') {
        return currentState;
      }

      return 'hiding';
    });
  }, []);

  const handleSubmit = useCallback(() => {
    if (!canSubmit) return;
    ask(query);
    setQuery('');
  }, [canSubmit, query, ask]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLInputElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  /** Auto-scroll response panel as streaming tokens arrive. */
  useEffect(() => {
    if (streamingContent && responseRef.current) {
      responseRef.current.scrollTop = responseRef.current.scrollHeight;
    }
  }, [streamingContent]);

  /**
   * Synchronizes the React animation state with Tauri-driven overlay visibility
   * requests emitted from the Rust backend.
   */
  useEffect(() => {
    let unlistenVisibility: (() => void) | undefined;

    const attachVisibilityListener = async () => {
      unlistenVisibility = await listen<OverlayVisibilityPayload>(
        OVERLAY_VISIBILITY_EVENT,
        ({ payload }) => {
          if (payload === 'show') {
            replayEntranceAnimation();
            return;
          }

          requestHideOverlay();
        },
      );
    };

    void attachVisibilityListener();

    return () => {
      unlistenVisibility?.();
    };
  }, [replayEntranceAnimation, requestHideOverlay]);

  /** Hide window on Escape or Cmd+W (macOS) / Ctrl+W. */
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (((e.metaKey || e.ctrlKey) && e.key === 'w') || e.key === 'Escape') {
        e.preventDefault();
        void invoke('notify_overlay_hidden');
        requestHideOverlay();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [requestHideOverlay]);

  /** Programmatic focus when the overlay becomes visible. */
  useEffect(() => {
    if (overlayState === 'visible') {
      const raf = requestAnimationFrame(() => inputRef.current?.focus());
      return () => cancelAnimationFrame(raf);
    }
  }, [overlayState]);

  /**
   * Commits the native window hide after a fixed deadline from the start of
   * the exit transition. Uses a timer rather than AnimatePresence.onExitComplete
   * because WKWebView throttles requestAnimationFrame in non-key windows,
   * causing spring animations to stall and the callback to never fire.
   */
  useEffect(() => {
    if (overlayState !== 'hiding') return;

    const timer = setTimeout(() => {
      void getCurrentWindow().hide();
      void invoke('notify_overlay_hidden');
      setOverlayState('hidden');
    }, HIDE_COMMIT_DELAY_MS);

    return () => clearTimeout(timer);
  }, [overlayState]);

  return (
    <div
      className="flex flex-col items-center justify-start h-screen w-screen p-10 bg-transparent overflow-visible"
      data-tauri-drag-region
    >
      <AnimatePresence mode="wait">
        {shouldRenderOverlay ? (
          <motion.div
            key="overlay"
            initial={{ opacity: 0, y: -20, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -16, scale: 0.98 }}
            transition={{ type: 'spring', stiffness: 260, damping: 24 }}
            className="w-full max-w-2xl px-4 py-2 overflow-visible"
          >
            {/* Input Bar Container — provides space for the shadow to bleed without clipping */}
            <div className="overflow-visible">
              <motion.div
                initial={{ opacity: 0, y: -12 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.04, duration: 0.22, ease: 'easeOut' }}
                className="flex items-center w-full bg-surface-base backdrop-blur-2xl rounded-2xl border border-surface-border shadow-bar p-1.5 gap-2"
              >
                <motion.img
                  src="/thuki-logo.png"
                  alt="Thuki"
                  className="w-10 h-10 shrink-0 rounded-xl"
                  initial={{ opacity: 0, scale: 0.8 }}
                  animate={{ opacity: 1, scale: 1 }}
                  transition={{ delay: 0.1, type: 'spring', stiffness: 300 }}
                  draggable={false}
                />

                <input
                  ref={inputRef}
                  type="text"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={handleKeyDown}
                  disabled={isGenerating}
                  autoFocus
                  placeholder="Ask Thuki anything..."
                  className="flex-1 min-w-0 bg-transparent border-none outline-none text-text-primary text-sm placeholder:text-text-secondary py-2 px-1 disabled:opacity-50"
                />

                <motion.button
                  type="button"
                  onClick={handleSubmit}
                  disabled={!canSubmit && !isGenerating}
                  whileHover={canSubmit ? { scale: 1.08 } : undefined}
                  whileTap={canSubmit ? { scale: 0.92 } : undefined}
                  className={`shrink-0 w-9 h-9 rounded-xl flex items-center justify-center transition-colors duration-200 ${
                    canSubmit
                      ? 'bg-primary text-neutral cursor-pointer'
                      : isGenerating
                        ? 'bg-surface-elevated text-primary cursor-default'
                        : 'bg-surface-elevated text-text-secondary cursor-default'
                  }`}
                  aria-label="Send message"
                >
                  {isGenerating ? <Spinner /> : ARROW_UP_ICON}
                </motion.button>
              </motion.div>

              {/* Response Panel — appears contextually below the bar */}
              <AnimatePresence>
                {showResponse ? (
                  <motion.div
                    initial={{ opacity: 0, y: -8 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0, y: -8 }}
                    transition={{ type: 'spring', stiffness: 300, damping: 28 }}
                    className="w-full mt-3 bg-surface-base backdrop-blur-2xl rounded-2xl border border-surface-border shadow-bar overflow-hidden"
                  >
                    <div
                      ref={responseRef}
                      className="p-4 max-h-85 overflow-y-auto text-sm leading-relaxed custom-scrollbar"
                    >
                      {displayContent ? (
                        <MarkdownRenderer content={displayContent} />
                      ) : isGenerating ? (
                        <div className="flex items-center gap-2 text-text-secondary">
                          <span className="w-1.5 h-1.5 rounded-full bg-primary animate-pulse" />
                          Thinking...
                        </div>
                      ) : null}
                      {error ? (
                        <p className="text-red-400 text-xs mt-3 p-2 rounded-lg bg-red-950/30 border border-red-900/50">
                          {error}
                        </p>
                      ) : null}
                    </div>
                  </motion.div>
                ) : null}
              </AnimatePresence>
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}

export default App;
