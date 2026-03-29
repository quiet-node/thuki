import { motion, AnimatePresence } from 'framer-motion';
import type React from 'react';
import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { useOllama } from './hooks/useOllama';
import { ConversationView } from './view/ConversationView';
import { AskBarView } from './view/AskBarView';
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
 * Main application orchestrator for Thuki.
 *
 * Implements an adaptive morphing UI container. It starts as a minimal spotlight-style
 * input bar (`AskBarView`), then smoothly transforms into a full chat window
 * (`ConversationView`) when the user sends their first message.
 *
 * This wrapper is strictly responsible for layout morphing, global hotkeys,
 * and window visibility state, delegating UI rendering logic to the view components.
 */
function App() {
  const [query, setQuery] = useState('');
  const [overlayState, setOverlayState] = useState<OverlayState>('hidden');
  const { messages, streamingContent, ask, isGenerating, error, reset } =
    useOllama();

  const inputRef = useRef<HTMLTextAreaElement>(null);

  /**
   * Session counter — incremented on each overlay open. Used in the motion
   * key to force AnimatePresence to fully unmount the stale tree before
   * mounting a fresh one, preventing a flash of the previous conversation.
   */
  const [sessionId, setSessionId] = useState(0);

  /**
   * Determines whether the UI has entered "chat mode" — i.e., the morphing
   * chat window state with message bubbles. Transitions from input-bar mode
   * to chat-window mode are animated via Framer Motion `layout` prop.
   */
  const isChatMode = messages.length > 0 || isGenerating;
  const shouldRenderOverlay = overlayState === 'visible';

  /**
   * Reference stored for ResizeObserver cleanup.
   */
  const observerRef = useRef<ResizeObserver | null>(null);

  /**
   * Callback ref to reliably attach the ResizeObserver when the conditionally
   * rendered Framer Motion container actually mounts in the DOM. This fixes
   * the bug where a standard useEffect would run before the DOM node was ready,
   * leaving the native window stuck at 600x700.
   */
  const setContainerRef = useCallback((node: HTMLDivElement | null) => {
    if (observerRef.current) {
      observerRef.current.disconnect();
      observerRef.current = null;
    }

    if (node) {
      const observer = new ResizeObserver((entries) => {
        requestAnimationFrame(() => {
          for (const entry of entries) {
            const rect = entry.target.getBoundingClientRect();
            // Total vertical room: 8px (pt-2) + 24px (pb-6) + 16px (motion py-2) = 48px.
            // This ensures the tightened drop shadows aren't clipped by the native window edge.
            const targetHeight = Math.ceil(rect.height) + 48;
            void getCurrentWindow().setSize(new LogicalSize(600, targetHeight));
          }
        });
      });

      observer.observe(node);
      observerRef.current = observer;
    }
  }, []);

  /**
   * Replays the entrance sequence by transitioning the overlay to the visible state.
   * Clears conversation state for a fresh session each time the overlay appears.
   */
  const replayEntranceAnimation = useCallback(() => {
    setSessionId((id) => id + 1);
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
    if (query.trim().length === 0 || isGenerating) return;
    ask(query);
    setQuery('');
    if (inputRef.current) {
      // Reset auto-grown textarea shrink back to 1 row
      inputRef.current.style.height = 'auto';
    }
  }, [query, isGenerating, ask]);

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

  /**
   * Combined close handler shared by the keyboard shortcut (Esc/Cmd+W)
   * and the traffic light close/minimize buttons. Notifies the Rust
   * backend and triggers the frontend exit animation sequence.
   */
  const handleCloseOverlay = useCallback(() => {
    void invoke('notify_overlay_hidden');
    requestHideOverlay();
  }, [requestHideOverlay]);

  /** Hide window on Escape or Cmd+W (macOS) / Ctrl+W. */
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (((e.metaKey || e.ctrlKey) && e.key === 'w') || e.key === 'Escape') {
        e.preventDefault();
        handleCloseOverlay();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [handleCloseOverlay]);

  /** Programmatic focus when the overlay becomes visible. */
  useEffect(() => {
    if (overlayState === 'visible') {
      const raf = requestAnimationFrame(() => inputRef.current?.focus());
      return () => cancelAnimationFrame(raf);
    }
  }, [overlayState]);

  /**
   * Commits the native window hide after a fixed deadline from the start of
   * the exit transition.
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

  /**
   * Handles mousedown on any surface of the application window.
   *
   * For non-interactive targets (transparent padding, container chrome, etc.):
   * - Calls `preventDefault()` to suppress the browser's default behaviour of
   *   blurring the active element, keeping textarea focus intact.
   * - Initiates a native platform drag via `startDragging()`.
   *
   * For interactive targets (textarea, buttons, links): returns early so
   * standard DOM behaviour (focus, click, selection) proceeds normally.
   */
  const handleDragStart = useCallback((e: React.MouseEvent) => {
    const INTERACTIVE_TAGS = new Set([
      'TEXTAREA',
      'INPUT',
      'BUTTON',
      'A',
      'SELECT',
      'PATH',
      'SVG',
    ]);
    let el = e.target as HTMLElement | null;
    while (el) {
      if (INTERACTIVE_TAGS.has(el.tagName.toUpperCase())) return;
      el = el.parentElement;
    }

    // Suppress the default mousedown side-effect (focus transfer / blur)
    // so the textarea retains keyboard input during window repositioning.
    e.preventDefault();
    void getCurrentWindow().startDragging();
  }, []);

  return (
    // Minimal padding (pt-2 pb-6) provides just enough physical clearance for the
    // tightened drop shadow to render without clipping at the native window edge.
    <div
      onMouseDown={handleDragStart}
      className="flex flex-col items-center justify-start h-screen w-screen px-3 pt-2 pb-6 bg-transparent overflow-visible"
    >
      <AnimatePresence mode="wait">
        {shouldRenderOverlay ? (
          <motion.div
            key={`overlay-${sessionId}`}
            initial={{ opacity: 0, y: -20, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -16, scale: 0.98 }}
            transition={{ type: 'spring', stiffness: 260, damping: 24 }}
            className="w-full max-w-2xl px-4 py-2 overflow-visible"
          >
            {/* Morphing Container — flex column ensures the input bar
                always sticks to the bottom without spring animation lag */}
            <div
              ref={setContainerRef}
              className={`morphing-container relative flex flex-col bg-surface-base backdrop-blur-2xl border border-surface-border overflow-hidden ${
                isChatMode
                  ? 'rounded-lg shadow-chat max-h-[600px]'
                  : 'rounded-2xl shadow-bar'
              }`}
            >
              {/* Chat Messages Area — morphs in when in chat mode */}
              <AnimatePresence>
                {isChatMode ? (
                  <ConversationView
                    messages={messages}
                    streamingContent={streamingContent}
                    isGenerating={isGenerating}
                    error={error}
                    onClose={handleCloseOverlay}
                  />
                ) : null}
              </AnimatePresence>

              {/* Input Bar — always pinned to the bottom */}
              <AskBarView
                query={query}
                setQuery={setQuery}
                isChatMode={isChatMode}
                isGenerating={isGenerating}
                onSubmit={handleSubmit}
                inputRef={inputRef}
              />
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}

export default App;
