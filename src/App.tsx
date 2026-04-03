import { motion, AnimatePresence } from 'framer-motion';
import type React from 'react';
import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { useOllama } from './hooks/useOllama';
import { useConversationHistory } from './hooks/useConversationHistory';
import { ConversationView } from './view/ConversationView';
import { AskBarView } from './view/AskBarView';
import { HistoryPanel } from './components/HistoryPanel';
import { quote } from './config';
import './App.css';

/** Ollama model used for this session — must match the Rust DEFAULT_MODEL_NAME. */
const MODEL_NAME = 'llama3.2:3b';

const OVERLAY_VISIBILITY_EVENT = 'thuki://visibility';

/**
 * Authoritative deadline from the start of the hide transition to the native
 * window hide call. Accounts for WKWebView `requestAnimationFrame` throttling
 * in non-key windows, which stalls spring animations indefinitely and makes
 * `AnimatePresence.onExitComplete` unreliable when the panel is unfocused.
 */
const HIDE_COMMIT_DELAY_MS = 350;

/** Must match `OVERLAY_LOGICAL_WIDTH` in `src-tauri/src/lib.rs`. */
const OVERLAY_WIDTH = 600;
/** Total transparent padding around the morphing container: pt-2(8) + pb-6(24) + motion py-2(16). */
const CONTAINER_VERTICAL_PADDING = 48;
/** Max morphing-container height in chat mode (matches `max-h-[600px]`) + vertical padding. */
const MAX_CHAT_WINDOW_HEIGHT = 600 + CONTAINER_VERTICAL_PADDING;

type WindowAnchor = { x: number; bottom_y: number; min_y: number };
type OverlayVisibilityPayload =
  | {
      state: 'show';
      selected_text: string | null;
      window_anchor: WindowAnchor | null;
    }
  | { state: 'hide-request' };
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

  /**
   * Whether the ask-bar history panel is currently open.
   * Distinct from the chat-mode history dropdown (controlled by the same toggle
   * but rendered differently based on `isChatMode`).
   */
  const [isHistoryOpen, setIsHistoryOpen] = useState(false);

  /**
   * Direct reference to the morphing container DOM node, stored alongside the
   * ResizeObserver so the dropdown sync effect can mutate `style.minHeight`
   * without going through React state (direct DOM mutation + CSS transition).
   */
  const morphingContainerNodeRef = useRef<HTMLDivElement | null>(null);

  const {
    conversationId,
    isSaved,
    save,
    persistTurn,
    loadConversation,
    deleteConversation,
    listConversations,
    reset: resetHistory,
  } = useConversationHistory();

  /**
   * Persist a completed user/assistant turn to SQLite if the conversation
   * has been saved. Passed as `onTurnComplete` to `useOllama`.
   */
  const handleTurnComplete = useCallback(
    async (
      userMsg: Parameters<typeof persistTurn>[0],
      assistantMsg: Parameters<typeof persistTurn>[1],
    ) => {
      await persistTurn(userMsg, assistantMsg);
    },
    [persistTurn],
  );

  const {
    messages,
    streamingContent,
    ask,
    cancel,
    isGenerating,
    error,
    reset,
    loadMessages,
  } = useOllama(handleTurnComplete);

  const inputRef = useRef<HTMLTextAreaElement>(null);

  /**
   * Session counter — incremented on each overlay open. Used in the motion
   * key to force AnimatePresence to fully unmount the stale tree before
   * mounting a fresh one, preventing a flash of the previous conversation.
   */
  const [sessionId, setSessionId] = useState(0);
  const [selectedContext, setSelectedContext] = useState<string | null>(null);

  /**
   * True when the window was spawned with an upward-growth anchor. Used to
   * flip the outer container to `justify-end` so the morphing container pins
   * to the bottom of the pre-expanded window and content grows upward.
   */
  const [isAnchoredUpward, setIsAnchoredUpward] = useState(false);

  /**
   * Determines whether the UI has entered "chat mode" — i.e., the morphing
   * chat window state with message bubbles. Transitions from input-bar mode
   * to chat-window mode are animated via Framer Motion `layout` prop.
   */
  const isChatMode = messages.length > 0 || isGenerating;

  /**
   * The bookmark save button is active once the AI has produced at least one
   * complete response. We check for an assistant message rather than any message
   * so the button never appears during the very first user-only half-turn.
   */
  const canSave = messages.some((m) => m.role === 'assistant');
  const shouldRenderOverlay = overlayState === 'visible';

  /**
   * Reference stored for ResizeObserver cleanup.
   */
  const observerRef = useRef<ResizeObserver | null>(null);

  /**
   * Holds the window anchor for the "above selection" spawn case.
   * Stored in a ref (not state) so the ResizeObserver closure can read the
   * latest value without needing to be recreated on each anchor change.
   */
  const windowAnchorRef = useRef<WindowAnchor | null>(null);

  /**
   * Set once the first ResizeObserver event has expanded the window to max
   * height for an anchored session. While true, all subsequent observer
   * events for the anchor path are skipped — the window stays at max and
   * content grows inside it. Reset when the anchor is cleared.
   */
  const isPreExpandedRef = useRef(false);

  /**
   * When the LLM starts generating and the window has an upward anchor, expand
   * immediately to max height before any streaming tokens arrive.
   *
   * Streamdown opens empty block elements (`<p></p>`) before their content,
   * causing the morphing container to grow in sudden steps. Each step triggers
   * a ResizeObserver → set_window_frame cycle that repositions the window
   * upward — visible as a jittery jump during upward-anchor sessions.
   *
   * Expanding to max height in a single `useEffect` call (before the first
   * token paint) gives the streaming text a fixed canvas to fill, eliminating
   * all incremental upward repositioning during the response.
   */
  useEffect(() => {
    if (!isGenerating || !windowAnchorRef.current || isPreExpandedRef.current)
      return;
    const anchor = windowAnchorRef.current;
    const maxHeight = Math.min(
      MAX_CHAT_WINDOW_HEIGHT,
      anchor.bottom_y - anchor.min_y,
    );
    const newY = anchor.bottom_y - maxHeight;
    isPreExpandedRef.current = true;
    void invoke('set_window_frame', {
      x: anchor.x,
      y: newY,
      width: OVERLAY_WIDTH,
      height: maxHeight,
    });
  }, [isGenerating]);

  /**
   * Callback ref to reliably attach the ResizeObserver when the conditionally
   * rendered Framer Motion container actually mounts in the DOM. This fixes
   * the bug where a standard useEffect would run before the DOM node was ready,
   * leaving the native window stuck at 600x700.
   *
   * When a window anchor is present (bar spawned above selection), the observer
   * also repositions the window upward to keep its bottom pinned to the anchor
   * as the conversation grows.
   */
  const setContainerRef = useCallback((node: HTMLDivElement | null) => {
    morphingContainerNodeRef.current = node;

    if (observerRef.current) {
      observerRef.current.disconnect();
      observerRef.current = null;
    }

    if (node) {
      const observer = new ResizeObserver(
        /* v8 ignore start -- ResizeObserver callback requires a native browser resize event */
        (entries) => {
          requestAnimationFrame(() => {
            for (const entry of entries) {
              const rect = entry.target.getBoundingClientRect();
              // Total vertical room: 8px (pt-2) + 24px (pb-6) + 16px (motion py-2) = 48px.
              // This ensures the tightened drop shadows aren't clipped by the native window edge.
              const targetHeight =
                Math.ceil(rect.height) + CONTAINER_VERTICAL_PADDING;
              const anchor = windowAnchorRef.current;
              if (anchor) {
                // Once the window has reached max height for this anchor
                // session, skip all further adjustments — content scrolls
                // internally inside the fixed-size window.
                if (isPreExpandedRef.current) return;

                const maxHeight = Math.min(
                  MAX_CHAT_WINDOW_HEIGHT,
                  anchor.bottom_y - anchor.min_y,
                );
                const neededHeight = Math.min(targetHeight, maxHeight);

                // Lock the observer once max height is reached.
                if (neededHeight >= maxHeight) {
                  isPreExpandedRef.current = true;
                }

                // Grow upward incrementally: pin the window bottom to the
                // anchor and expand the top edge as content grows. Because
                // `set_window_frame` applies position + size atomically on
                // the main thread, there is no inter-frame jitter.
                const newY = anchor.bottom_y - neededHeight;
                void invoke('set_window_frame', {
                  x: anchor.x,
                  y: newY,
                  width: OVERLAY_WIDTH,
                  height: neededHeight,
                });
              } else {
                void getCurrentWindow().setSize(
                  new LogicalSize(OVERLAY_WIDTH, targetHeight),
                );
              }
            }
          });
        },
        /* v8 ignore stop */
      );

      observer.observe(node);
      observerRef.current = observer;
    }
  }, []);

  /**
   * Replays the entrance sequence by transitioning the overlay to the visible state.
   * Clears conversation state for a fresh session each time the overlay appears.
   */
  const replayEntranceAnimation = useCallback(
    (context: string | null, anchor: WindowAnchor | null) => {
      windowAnchorRef.current = anchor;
      isPreExpandedRef.current = false;
      setIsAnchoredUpward(anchor !== null);
      setSessionId((id) => id + 1);
      setQuery('');
      setSelectedContext(context);
      setIsHistoryOpen(false);
      reset();
      resetHistory();
      setOverlayState('visible');
    },
    [reset, resetHistory],
  );

  /**
   * Moves the overlay into an exit phase. The actual Tauri window hide call is
   * deferred until Framer Motion finishes the exit transition.
   */
  const requestHideOverlay = useCallback(() => {
    windowAnchorRef.current = null;
    isPreExpandedRef.current = false;
    setSelectedContext(null);
    setOverlayState((currentState) => {
      if (currentState === 'hidden' || currentState === 'hiding') {
        return currentState;
      }
      return 'hiding';
    });
  }, []);

  /** Ref attached to the chat-mode history dropdown for click-outside detection. */
  const historyDropdownRef = useRef<HTMLDivElement>(null);

  /** Toggles the history panel open/closed. */
  const handleHistoryToggle = useCallback(() => {
    setIsHistoryOpen((prev) => !prev);
  }, []);

  /**
   * Close the chat-mode history dropdown when the user clicks outside it.
   * Clicks on the toggle button itself are excluded so the button's own
   * onClick handler (handleHistoryToggle) can manage the toggle normally.
   */
  useEffect(() => {
    if (!(isChatMode && isHistoryOpen)) return;

    const handleMouseDown = (e: MouseEvent) => {
      const target = e.target as Element;
      if (
        historyDropdownRef.current?.contains(target) ||
        target.closest?.('[data-history-toggle]')
      ) {
        return;
      }
      setIsHistoryOpen(false);
    };

    document.addEventListener('mousedown', handleMouseDown);
    return () => document.removeEventListener('mousedown', handleMouseDown);
  }, [isChatMode, isHistoryOpen]);

  /**
   * Observes the dropdown's height while it's open and mutates the morphing
   * container's `min-height` style directly (bypassing React state) so the
   * native window grows exactly as tall as the dropdown needs. A CSS transition
   * on the container drives the smooth resize; the existing ResizeObserver fires
   * per-frame and calls `setSize()` as the transition runs.
   *
   * Direct DOM mutation avoids the React state → Framer Motion → ResizeObserver
   * indirect chain that broke timing. ResizeObserver tracks async conversation
   * list load so `min-height` stays accurate as content populates.
   */
  useEffect(() => {
    /* v8 ignore start -- ResizeObserver + DOM mutations require a real browser */
    if (!isChatMode || !isHistoryOpen) {
      if (morphingContainerNodeRef.current) {
        morphingContainerNodeRef.current.style.minHeight = '';
      }
      return;
    }

    const dropdown = historyDropdownRef.current;
    const container = morphingContainerNodeRef.current;
    if (!dropdown || !container) return;

    const sync = () => {
      container.style.minHeight = `${dropdown.offsetTop + dropdown.offsetHeight + 8}px`;
    };

    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(dropdown);
    return () => ro.disconnect();
    /* v8 ignore stop */
  }, [isChatMode, isHistoryOpen]);

  /** Saves the current conversation to SQLite. */
  const handleSave = useCallback(async () => {
    try {
      await save(messages, MODEL_NAME);
    } catch {
      // Save failed — bookmark state stays unchanged; the error is surfaced by
      // the Tauri runtime. No UI banner here; save is a user-initiated fire-and-
      // forget action with visible feedback via the bookmark icon state.
    }
  }, [save, messages]);

  /**
   * Loads a conversation from history, replacing the current session.
   *
   * Closes the history panel regardless of success or failure: on success the
   * loaded messages replace the current session; on failure the current session
   * is preserved and the panel is dismissed so the user is not left in a
   * half-open state.
   */
  const handleLoadConversation = useCallback(
    async (id: string) => {
      try {
        const loaded = await loadConversation(id);
        loadMessages(loaded);
      } catch {
        // Load failed — current session is preserved intact.
      } finally {
        setIsHistoryOpen(false);
      }
    },
    [loadConversation, loadMessages],
  );

  /**
   * Saves the current unsaved session then loads the requested conversation.
   *
   * If save fails the operation is aborted — we do not load the target
   * conversation because the current session has not been persisted yet.
   * If save succeeds but load fails the panel is still dismissed; the
   * current session has been saved so no data is lost.
   */
  const handleSaveAndLoad = useCallback(
    async (id: string) => {
      try {
        await save(messages, MODEL_NAME);
      } catch {
        // Save failed — abort to avoid leaving the current session unprotected.
        return;
      }
      try {
        const loaded = await loadConversation(id);
        loadMessages(loaded);
      } catch {
        // Load failed — save already committed; dismiss panel, keep current view.
      } finally {
        setIsHistoryOpen(false);
      }
    },
    [save, messages, loadConversation, loadMessages],
  );

  /**
   * Deletes a conversation from the history panel.
   *
   * When the deleted conversation is the currently active one, both the
   * message history (`reset`) and the persistence state (`resetHistory`) are
   * cleared so the UI returns to the blank ask-bar state. The error is
   * re-thrown so `HistoryPanel` can roll back its optimistic removal.
   */
  const handleDeleteConversation = useCallback(
    async (id: string) => {
      await deleteConversation(id);
      if (id === conversationId) {
        reset();
        resetHistory();
      }
    },
    [deleteConversation, conversationId, reset, resetHistory],
  );

  /** Starts a fresh conversation from within conversation view. */
  const handleNewConversation = useCallback(() => {
    reset();
    resetHistory();
    setIsHistoryOpen(false);
    setQuery('');
  }, [reset, resetHistory]);

  const handleSubmit = useCallback(() => {
    if (query.trim().length === 0 || isGenerating) return;
    // Sanitize externally-sourced context: strip control characters and enforce
    // a length cap to limit prompt-injection surface from host-app selections.
    // eslint-disable-next-line no-control-regex
    const CONTROL_CHARS = /[\x00-\x08\x0b\x0c\x0e-\x1f]/g;
    const sanitized = selectedContext
      ?.replace(CONTROL_CHARS, '')
      .slice(0, quote.maxContextLength);
    const hasContext = sanitized && sanitized.trim().length > 0;
    ask(query, hasContext ? sanitized : undefined);
    setSelectedContext(null);
    setQuery('');
    if (inputRef.current) {
      inputRef.current.style.height = 'auto';
    }
  }, [query, isGenerating, ask, selectedContext, setSelectedContext]);

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
          if (payload.state === 'show') {
            replayEntranceAnimation(
              payload.selected_text ?? null,
              payload.window_anchor ?? null,
            );
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
    const el = e.target as HTMLElement | null;

    // 1. Allow native text selection in explicitly selectable regions.
    // If the click occurs inside a chat bubble (which has .select-text),
    // we return early so the user can highlight and copy the text.
    if (el?.closest('.select-text')) {
      return;
    }

    // 2. Allow interaction with standard interactive elements.
    const INTERACTIVE_TAGS = new Set([
      'TEXTAREA',
      'INPUT',
      'BUTTON',
      'A',
      'SELECT',
      'PATH',
      'SVG',
    ]);
    let current = el;
    while (current) {
      if (INTERACTIVE_TAGS.has(current.tagName.toUpperCase())) return;
      current = current.parentElement;
    }

    // Suppress the default mousedown side-effect (focus transfer / blur)
    // so the textarea retains keyboard input during window repositioning.
    e.preventDefault();
    void getCurrentWindow().startDragging();

    // After the user repositions the window, drop the upward-grow anchor so
    // subsequent conversation growth tracks the new position downward.
    window.addEventListener(
      'mouseup',
      () => {
        windowAnchorRef.current = null;
        isPreExpandedRef.current = false;
        setIsAnchoredUpward(false);
      },
      { once: true },
    );
  }, []);

  return (
    // Minimal padding (pt-2 pb-6) provides just enough physical clearance for the
    // tightened drop shadow to render without clipping at the native window edge.
    <div
      onMouseDown={handleDragStart}
      className={`flex flex-col items-center ${isAnchoredUpward ? 'justify-end' : 'justify-start'} h-screen w-screen px-3 pt-2 pb-6 bg-transparent overflow-visible`}
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
            {/* Relative wrapper — serves as the positioning context for the
                chat-mode history dropdown so it can sit outside the morphing
                container's overflow-hidden boundary without being clipped. */}
            <div className="relative">
              {/* Morphing Container — flex column ensures the input bar
                  always sticks to the bottom without spring animation lag.
                  A CSS `transition: min-height` drives smooth window growth
                  when the chat-mode history dropdown is open; the existing
                  ResizeObserver fires per-frame and calls setSize() so the
                  native window tracks the animation. The dropdown is a sibling
                  (not a child) so overflow-hidden never clips it. */}
              <div
                ref={setContainerRef}
                style={{
                  transition: 'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)',
                }}
                className={`morphing-container relative flex flex-col bg-surface-base backdrop-blur-2xl border border-surface-border ${
                  isChatMode
                    ? 'rounded-lg shadow-chat max-h-[600px] overflow-hidden'
                    : 'rounded-2xl shadow-bar overflow-hidden'
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
                      onSave={handleSave}
                      isSaved={isSaved}
                      canSave={canSave}
                      onHistoryOpen={handleHistoryToggle}
                    />
                  ) : null}
                </AnimatePresence>

                {/* Ask-bar mode history panel — inline below the input bar.
                    The !isChatMode gate lives OUTSIDE AnimatePresence so that when
                    a conversation is loaded (isChatMode → true) the panel unmounts
                    instantly — no exit animation runs alongside ConversationView
                    mounting. Without this, AnimatePresence would hold the panel in
                    the DOM during its exit while ConversationView is also present,
                    causing two rapid ResizeObserver → setSize() calls (jitter).
                    AnimatePresence is still used for the manual toggle (isHistoryOpen)
                    so the drawer height-animates smoothly open and closed. */}
                {!isChatMode && (
                  <AnimatePresence>
                    {isHistoryOpen ? (
                      <motion.div
                        key="ask-bar-history"
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        transition={{
                          height: { duration: 0.25, ease: [0.16, 1, 0.3, 1] },
                          opacity: { duration: 0.2 },
                        }}
                        style={{ overflow: 'hidden' }}
                        className="border-t border-surface-border"
                      >
                        <HistoryPanel
                          listConversations={listConversations}
                          onLoadConversation={handleLoadConversation}
                          onSaveAndLoad={handleSaveAndLoad}
                          onDeleteConversation={handleDeleteConversation}
                          hasCurrentMessages={false}
                          showNewConversation={false}
                          currentConversationId={conversationId}
                        />
                      </motion.div>
                    ) : null}
                  </AnimatePresence>
                )}

                {/* Input Bar — always pinned to the bottom */}
                <AskBarView
                  query={query}
                  setQuery={setQuery}
                  isChatMode={isChatMode}
                  isGenerating={isGenerating}
                  onSubmit={handleSubmit}
                  onCancel={cancel}
                  inputRef={inputRef}
                  selectedText={selectedContext ?? undefined}
                  onHistoryOpen={handleHistoryToggle}
                />
              </div>

              {/* Chat-mode history dropdown — sibling of the morphing container so
                  it is never clipped by its overflow-hidden. Positioned absolutely
                  within this relative wrapper (same coordinate space as the
                  container). The container's minHeight animation grows the native
                  window tall enough to reveal the full dropdown. */}
              <AnimatePresence>
                {isChatMode && isHistoryOpen ? (
                  <motion.div
                    ref={historyDropdownRef}
                    key="chat-history"
                    initial={{ opacity: 0, y: -8, scale: 0.97 }}
                    animate={{ opacity: 1, y: 0, scale: 1 }}
                    exit={{ opacity: 0, y: -8, scale: 0.97 }}
                    transition={{ type: 'spring', stiffness: 400, damping: 30 }}
                    className="history-dropdown absolute right-3 top-10 z-50 w-56 rounded-xl border border-surface-border bg-surface-base shadow-chat overflow-hidden flex flex-col"
                  >
                    <HistoryPanel
                      listConversations={listConversations}
                      onLoadConversation={handleLoadConversation}
                      onSaveAndLoad={handleSaveAndLoad}
                      onDeleteConversation={handleDeleteConversation}
                      hasCurrentMessages={messages.length > 0 && !isSaved}
                      currentConversationId={conversationId}
                      showNewConversation={true}
                      onNewConversation={handleNewConversation}
                    />
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
