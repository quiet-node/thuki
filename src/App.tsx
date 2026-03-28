import { motion, AnimatePresence } from 'framer-motion';
import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { useOllama } from './hooks/useOllama';
import { ChatBubble } from './components/ChatBubble';
import { TypingIndicator } from './components/TypingIndicator';
import { WindowControls } from './components/WindowControls';
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
 * Implements an adaptive morphing UI: starts as a minimal spotlight-style input
 * bar, then smoothly transforms into a full chat window when the user sends
 * their first message. Uses Framer Motion's `layout` animations for seamless
 * container morphing with GPU-accelerated transforms and spring physics.
 */
function App() {
  const [query, setQuery] = useState('');
  const [overlayState, setOverlayState] = useState<OverlayState>('hidden');
  const { messages, streamingContent, ask, isGenerating, error, reset } =
    useOllama();
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  /**
   * Session counter — incremented on each overlay open. Used in the motion
   * key to force AnimatePresence to fully unmount the stale tree before
   * mounting a fresh one, preventing a flash of the previous conversation.
   * Must be state (not a ref) because it is read during render for the key.
   */
  const [sessionId, setSessionId] = useState(0);

  const canSubmit = query.trim().length > 0 && !isGenerating;

  /**
   * Determines whether the UI has entered "chat mode" — i.e., the morphing
   * chat window state with message bubbles. Transitions from input-bar mode
   * to chat-window mode are animated via Framer Motion `layout` prop.
   */
  const isChatMode = messages.length > 0 || isGenerating;

  const shouldRenderOverlay = overlayState === 'visible';

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
    if (!canSubmit) return;
    ask(query);
    setQuery('');
    if (inputRef.current) {
      inputRef.current.style.height = 'auto';
    }
  }, [canSubmit, query, ask]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  /**
   * Auto-resizes the textarea to fit its content up to a maximum height.
   * Uses scrollHeight (layout read) followed by a style write — single
   * forced reflow per input event, which is unavoidable for auto-grow.
   */
  const handleTextareaChange = useCallback(
    (e: React.ChangeEvent<HTMLTextAreaElement>) => {
      setQuery(e.target.value);
      const el = e.target;
      el.style.height = 'auto';
      el.style.height = `${Math.min(el.scrollHeight, 144)}px`;
    },
    [],
  );

  /**
   * Tracks whether the user is "pinned" near the bottom of the scroll
   * container. When pinned, new streaming tokens auto-scroll the view.
   * When the user manually scrolls up, pinning is released so they can
   * read older messages undisturbed — identical to ChatGPT's behavior.
   */
  const isUserNearBottomRef = useRef(true);

  /** Threshold in pixels — if within this distance of the bottom, consider "pinned". */
  const NEAR_BOTTOM_THRESHOLD = 60;

  /**
   * Scroll event handler — updates the pinned state based on the user's
   * current scroll position relative to the bottom of the container.
   */
  const handleScroll = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const { scrollTop, scrollHeight, clientHeight } = container;
    isUserNearBottomRef.current =
      scrollHeight - scrollTop - clientHeight < NEAR_BOTTOM_THRESHOLD;
  }, []);

  /**
   * Auto-scroll the chat container to the bottom — but only when the user
   * is pinned near the bottom. This lets users scroll up to read older
   * messages while streaming continues without yanking them back down.
   */
  useEffect(() => {
    if (!isUserNearBottomRef.current) return;

    const container = scrollContainerRef.current;
    if (!container) return;

    const raf = requestAnimationFrame(() => {
      container.scrollTop = container.scrollHeight;
    });

    return () => cancelAnimationFrame(raf);
  }, [messages, streamingContent]);

  /**
   * Re-pin to bottom whenever the user sends a new message.
   * This ensures the view follows the AI response for the new query.
   */
  useEffect(() => {
    if (messages.length > 0) {
      const lastMsg = messages[messages.length - 1];
      if (lastMsg.role === 'user') {
        isUserNearBottomRef.current = true;
      }
    }
  }, [messages]);

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

  /**
   * Initiates native window dragging when the user mousedowns on any
   * non-interactive surface of the morphing container.
   *
   * Uses `getCurrentWindow().startDragging()` instead of the declarative
   * `data-tauri-drag-region` attribute, which only works on the exact
   * element it's applied to — not on children. This approach lets the
   * entire visible surface (chat bubbles, padding, separator) be draggable
   * while preserving interactivity for textarea, buttons, and links.
   */
  const handleDragStart = useCallback((e: React.MouseEvent) => {
    const INTERACTIVE_TAGS = new Set([
      'TEXTAREA',
      'INPUT',
      'BUTTON',
      'A',
      'SELECT',
    ]);
    let el = e.target as HTMLElement | null;
    while (el) {
      if (INTERACTIVE_TAGS.has(el.tagName)) return;
      el = el.parentElement;
    }
    void getCurrentWindow().startDragging();
  }, []);

  return (
    <div className="flex flex-col items-center justify-start h-screen w-screen p-10 bg-transparent overflow-visible">
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
              className={`morphing-container relative flex flex-col bg-surface-base backdrop-blur-2xl border border-surface-border overflow-hidden ${
                isChatMode ? 'rounded-lg shadow-chat max-h-[calc(100vh-9rem)]' : 'rounded-2xl shadow-bar'
              }`}
            >
              {/* Chat Messages Area — renders when in chat mode */}
              <AnimatePresence>
                {isChatMode ? (
                  <motion.div
                    key="chat-area"
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    transition={{ opacity: { duration: 0.2 } }}
                    className="chat-area flex-1 min-h-0 flex flex-col"
                  >
                    {/* Traffic light window controls + logo */}
                    <WindowControls
                      onClose={handleCloseOverlay}
                      onDragStart={handleDragStart}
                    />
                    <div
                      ref={scrollContainerRef}
                      onScroll={handleScroll}
                      className="chat-messages-scroll px-5 py-4 flex flex-col gap-3 flex-1 min-h-0 overflow-y-auto select-text"
                    >
                      {messages.map((msg, i) => (
                        <ChatBubble
                          key={`${msg.role}-${i}`}
                          role={msg.role}
                          content={msg.content}
                          index={i}
                        />
                      ))}

                      {/* Streaming AI response — renders as a live-updating bubble */}
                      {streamingContent ? (
                        <ChatBubble
                          key="streaming"
                          role="assistant"
                          content={streamingContent}
                          index={messages.length}
                        />
                      ) : null}

                      {/* Typing indicator — shows before any tokens arrive */}
                      {isGenerating && !streamingContent ? (
                        <TypingIndicator />
                      ) : null}

                      {/* Error display */}
                      {error ? (
                        <motion.div
                          initial={{ opacity: 0, y: 6 }}
                          animate={{ opacity: 1, y: 0 }}
                          className="flex w-full justify-start"
                        >
                          <p className="text-red-400 text-xs px-4 py-2.5 rounded-2xl rounded-bl-md bg-red-950/30 border border-red-900/50 max-w-[80%]">
                            {error}
                          </p>
                        </motion.div>
                      ) : null}
                    </div>

                    {/* Separator line between messages and input */}
                    <motion.div
                      initial={{ opacity: 0, scaleX: 0 }}
                      animate={{ opacity: 1, scaleX: 1 }}
                      transition={{ delay: 0.15, duration: 0.3 }}
                      className="h-px bg-surface-border origin-center"
                    />
                  </motion.div>
                ) : null}
              </AnimatePresence>

              {/* Input Bar — shrink-0 pins it to the bottom. Also serves
                  as the drag handle for moving the window (industry standard:
                  toolbar = drag area, content area = text-selectable). */}
              <div
                onMouseDown={handleDragStart}
                className="flex items-center w-full px-3 py-2.5 gap-2 shrink-0"
              >
                {/* Logo — smoothly transitions between large (input-bar) and compact (chat) */}
                <img
                  src="/thuki-logo.png"
                  alt="Thuki"
                  className={`shrink-0 transition-all duration-300 ease-out ${
                    isChatMode ? 'w-6 h-6 rounded-lg' : 'w-10 h-10 rounded-xl'
                  }`}
                  draggable={false}
                />

                <textarea
                  ref={inputRef}
                  value={query}
                  onChange={handleTextareaChange}
                  onKeyDown={handleKeyDown}
                  disabled={isGenerating}
                  autoFocus
                  rows={1}
                  placeholder={
                    isChatMode ? 'Reply...' : 'Ask Thuki anything...'
                  }
                  className="flex-1 min-w-0 bg-transparent border-none outline-none text-text-primary text-sm placeholder:text-text-secondary py-2 px-1 disabled:opacity-50 resize-none leading-relaxed"
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
              </div>
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
    </div>
  );
}

export default App;
