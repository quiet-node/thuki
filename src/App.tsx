import { motion, AnimatePresence } from 'framer-motion';
import type React from 'react';
import { useState, useEffect, useCallback, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { useOllama } from './hooks/useOllama';
import type { Message } from './hooks/useOllama';
import { useConversationHistory } from './hooks/useConversationHistory';
import { ConversationView } from './view/ConversationView';
import { AskBarView, MAX_IMAGES } from './view/AskBarView';
import { OnboardingView } from './view/OnboardingView';
import { HistoryPanel } from './components/HistoryPanel';
import { ImagePreviewModal } from './components/ImagePreviewModal';
import type { AttachedImage } from './types/image';
import { quote } from './config';
import { SCREEN_CAPTURE_PLACEHOLDER } from './config/commands';
import './App.css';

/** Ollama model used for this session — must match the Rust DEFAULT_MODEL_NAME. */
const MODEL_NAME = 'gemma3:4b';

const OVERLAY_VISIBILITY_EVENT = 'thuki://visibility';
const ONBOARDING_EVENT = 'thuki://onboarding';

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
  /** True once the backend signals that one or more permissions are missing. */
  const [showOnboarding, setShowOnboarding] = useState(false);

  /**
   * Whether the ask-bar history panel is currently open.
   * Distinct from the chat-mode history dropdown (controlled by the same toggle
   * but rendered differently based on `isChatMode`).
   */
  const [isHistoryOpen, setIsHistoryOpen] = useState(false);
  /**
   * True when the user clicked + while an unsaved conversation is active.
   * Causes the history dropdown to show a SwitchConfirmation prompt instead
   * of the conversation list.
   */
  const [pendingNewConversation, setPendingNewConversation] = useState(false);

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
    unsave,
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
    reset,
    loadMessages,
  } = useOllama(handleTurnComplete);

  const inputRef = useRef<HTMLTextAreaElement>(null);

  /** Images attached to the current (unsent) message. Blob URLs render
   *  immediately; file paths are set asynchronously after Rust processing. */
  const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
  /** URL of the image currently open in the preview modal (blob or asset URL). */
  const [previewImageUrl, setPreviewImageUrl] = useState<string | null>(null);

  /** When the user submits while images are still processing, the submit
   *  intent is stored here. The effect below watches `attachedImages` and
   *  fires the actual `ask()` once every image has a resolved `filePath`. */
  const pendingSubmitRef = useRef<{
    query: string;
    context: string | undefined;
  } | null>(null);
  /** True while waiting for images to finish processing before a deferred
   *  submit. Drives the "waiting" UI state in the ask bar. */
  const [isSubmitPending, setIsSubmitPending] = useState(false);
  /** Error message from a failed /screen capture. Shown inline above the ask
   *  bar so the user knows capture failed rather than seeing no response. */
  const [captureError, setCaptureError] = useState<string | null>(null);
  /**
   * Set to true when a /screen capture is dispatched, false when it resolves
   * or when the user cancels. Lets the async tail in handleScreenSubmit
   * detect a mid-flight cancellation and skip the ask() call.
   */
  const screenCapturePendingRef = useRef(false);
  /**
   * Stores the input state (query + context) captured just before a /screen
   * submit clears them. Used by handleCancel to restore the ask bar if the
   * user aborts the in-flight capture.
   */
  const screenCaptureInputSnapshotRef = useRef<{
    query: string;
    context: string | undefined;
  } | null>(null);
  /** User message shown in the chat while waiting for images to finish
   *  processing. Cleared when `ask()` fires and adds the real message. */
  const [pendingUserMessage, setPendingUserMessage] = useState<Message | null>(
    null,
  );

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
  const isChatMode = messages.length > 0 || isGenerating || isSubmitPending;

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
   * Ref attached to the outermost layout div. Used to set an explicit
   * `minHeight` before calling `set_window_frame` in the anchor path so the
   * CSS layout matches the new window dimensions before WKWebView's viewport
   * size event arrives — preventing the one-frame flash where `h-screen` is
   * still the old small height but the window has already repositioned upward.
   */
  const outerContainerRef = useRef<HTMLDivElement | null>(null);

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
    // Pre-set CSS min-height so justify-end positions correctly during the
    // WKWebView viewport update lag that follows set_window_frame.
    /* v8 ignore start -- DOM ref null guard: always set when overlay is visible */
    if (outerContainerRef.current) {
      outerContainerRef.current.style.minHeight = `${maxHeight}px`;
    }
    /* v8 ignore stop */
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

                // Pre-set CSS min-height before the native resize so the
                // WKWebView layout is correct during its viewport update lag.
                if (outerContainerRef.current) {
                  outerContainerRef.current.style.minHeight = `${neededHeight}px`;
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
      /* v8 ignore start -- DOM ref null guard: always set when overlay is visible */
      if (outerContainerRef.current) {
        outerContainerRef.current.style.minHeight = '';
      }
      /* v8 ignore stop */
      setIsAnchoredUpward(anchor !== null);
      setSessionId((id) => id + 1);
      setQuery('');
      setSelectedContext(context);
      setIsHistoryOpen(false);
      setAttachedImages((prev) => {
        for (const img of prev) URL.revokeObjectURL(img.blobUrl);
        return [];
      });
      pendingSubmitRef.current = null;
      screenCapturePendingRef.current = false;
      screenCaptureInputSnapshotRef.current = null;
      setIsSubmitPending(false);
      setPendingUserMessage(null);
      setCaptureError(null);

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
    /* v8 ignore start -- DOM ref null guard: always set when overlay is visible */
    if (outerContainerRef.current) {
      outerContainerRef.current.style.minHeight = '';
    }
    /* v8 ignore stop */
    screenCapturePendingRef.current = false;
    screenCaptureInputSnapshotRef.current = null;
    setSelectedContext(null);
    setPreviewImageUrl(null);
    setAttachedImages((prev) => {
      for (const img of prev) URL.revokeObjectURL(img.blobUrl);
      return [];
    });
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

  // Clear any pending new-conversation confirmation whenever the panel closes.
  // Uses a ref-based approach to avoid the @eslint-react/set-state-in-effect
  // warning from calling setState synchronously inside an effect body.
  const prevHistoryOpenRef = useRef(isHistoryOpen);
  if (prevHistoryOpenRef.current && !isHistoryOpen) {
    setPendingNewConversation(false);
  }
  prevHistoryOpenRef.current = isHistoryOpen;

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

  /**
   * Toggles the save state of the current conversation.
   * - Not saved → saves to SQLite (bookmark fills).
   * - Already saved → deletes from SQLite, marks unsaved (bookmark empties);
   *   messages remain in the UI so the session can be re-saved if desired.
   */
  const handleSave = useCallback(async () => {
    try {
      if (isSaved) {
        await unsave();
      } else {
        await save(messages, MODEL_NAME);
      }
    } catch {
      // State stays unchanged on failure; feedback is implicit in the icon.
    }
  }, [isSaved, unsave, save, messages]);

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
   * When the deleted conversation is the currently active one, only the
   * persistence state (`resetHistory`) is cleared — messages remain visible
   * so the user can continue chatting or re-save. The error is intentionally
   * re-thrown so `HistoryPanel` can roll back its optimistic removal.
   */
  const handleDeleteConversation = useCallback(
    async (id: string) => {
      await deleteConversation(id);
      if (id === conversationId) {
        resetHistory();
      }
    },
    [deleteConversation, conversationId, resetHistory],
  );

  /**
   * Shared reset sequence for all "start a new conversation" paths.
   *
   * Mirrors what `replayEntranceAnimation` does for the anchor-mode state so
   * the Tauri window shrinks back to ask-bar height regardless of whether the
   * session was launched from a text-selection anchor:
   *
   * - `isPreExpandedRef.current = false` unblocks the ResizeObserver in anchor
   *   mode so it can call `set_window_frame` with the (smaller) ask-bar height.
   * - Clearing `outerContainerRef.current.style.minHeight` removes the inline
   *   CSS constraint that was keeping the outer container at the expanded height.
   */
  const resetForNewConversation = useCallback(() => {
    isPreExpandedRef.current = false;
    /* v8 ignore start -- DOM ref null guard */
    if (outerContainerRef.current) {
      outerContainerRef.current.style.minHeight = '';
    }
    /* v8 ignore stop */
    reset();
    resetHistory();
    setIsHistoryOpen(false);
    setQuery('');
    setAttachedImages((prev) => {
      for (const img of prev) URL.revokeObjectURL(img.blobUrl);
      return [];
    });
    pendingSubmitRef.current = null;
    screenCapturePendingRef.current = false;
    screenCaptureInputSnapshotRef.current = null;
    setIsSubmitPending(false);
    setPendingUserMessage(null);
  }, [reset, resetHistory]);

  /**
   * Starts a fresh conversation from within conversation view.
   * If the current conversation has unsaved messages, opens the history
   * dropdown and surfaces a SwitchConfirmation prompt instead of resetting
   * immediately.
   */
  const handleNewConversation = useCallback(() => {
    if (!isSaved && messages.length > 0) {
      setPendingNewConversation(true);
      setIsHistoryOpen(true);
      return;
    }
    resetForNewConversation();
  }, [isSaved, messages.length, resetForNewConversation]);

  /** Saves the current conversation then starts a fresh one. */
  const handleSaveAndNew = useCallback(async () => {
    try {
      await save(messages, MODEL_NAME);
    } catch {
      return;
    }
    resetForNewConversation();
  }, [save, messages, resetForNewConversation]);

  /** Discards the current conversation and starts a fresh one. */
  const handleJustNew = useCallback(() => {
    resetForNewConversation();
  }, [resetForNewConversation]);

  /**
   * Handles newly attached image files. Creates blob URLs immediately for
   * instant thumbnail rendering, then processes each file in the background
   * via base64-encoded IPC to the Rust backend.
   */
  const handleImagesAttached = useCallback((files: File[]) => {
    const newImages: AttachedImage[] = files.map((file) => ({
      id: crypto.randomUUID(),
      blobUrl: URL.createObjectURL(file),
      filePath: null,
    }));

    setAttachedImages((prev) => [...prev, ...newImages]);

    // Defer backend processing to the next frame so React can render the
    // blob URL thumbnails immediately — keeps the UI responsive while
    // FileReader + IPC serialisation happen in subsequent event-loop ticks.
    requestAnimationFrame(() => {
      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        const imageId = newImages[i].id;

        const reader = new FileReader();
        reader.onload = () => {
          // Extract pure base64 from the data URL (strip "data:image/png;base64,").
          const base64 = (reader.result as string).split(',')[1];
          invoke<string>('save_image_command', { imageDataBase64: base64 })
            .then((filePath) => {
              setAttachedImages((prev) =>
                prev.map((img) =>
                  img.id === imageId ? { ...img, filePath } : img,
                ),
              );
            })
            .catch(() => {
              setAttachedImages((prev) => {
                for (const img of prev) {
                  if (img.id === imageId) URL.revokeObjectURL(img.blobUrl);
                }
                return prev.filter((img) => img.id !== imageId);
              });
            });
        };
        reader.readAsDataURL(file);
      }
    });
  }, []);

  /**
   * Invokes the Rust `capture_screenshot` command, which hides the window,
   * lets the user drag-select a screen region, then returns the captured image
   * as a base64 PNG string (or null if the user cancelled).
   * On success, converts the base64 to a File and feeds it into the existing
   * handleImagesAttached pipeline — identical to a paste or drag-drop.
   */
  const handleScreenshot = useCallback(async () => {
    /* v8 ignore start -- defensive guard: button is always disabled at max images, so this branch is unreachable through normal UI interaction */
    if (attachedImages.length >= MAX_IMAGES) return;
    /* v8 ignore stop */
    const base64 = await invoke<string | null>('capture_screenshot_command');
    if (!base64) return;
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    const blob = new Blob([bytes], { type: 'image/png' });
    const file = new File([blob], 'screenshot.png', { type: 'image/png' });
    handleImagesAttached([file]);
  }, [attachedImages, handleImagesAttached]);

  /** Removes an attached image from state, revokes the blob URL, and
   *  deletes the staged file from disk if processing completed. */
  const handleImageRemove = useCallback((id: string) => {
    setAttachedImages((prev) => {
      const img = prev.find((i) => i.id === id);
      if (img) {
        URL.revokeObjectURL(img.blobUrl);
        if (img.filePath) {
          void invoke('remove_image_command', { path: img.filePath });
        }
      }
      return prev.filter((i) => i.id !== id);
    });
  }, []);

  /** Opens the preview modal for an attached image (identified by ID).
   *  The ID always comes from the thumbnail component which only renders
   *  items present in attachedImages, so the find always succeeds. */
  const handleAskBarImagePreview = useCallback(
    (id: string) => {
      setPreviewImageUrl(attachedImages.find((i) => i.id === id)!.blobUrl);
    },
    [attachedImages],
  );

  /** Opens the preview modal for a chat history image (identified by file path). */
  const handleChatImagePreview = useCallback((path: string) => {
    setPreviewImageUrl(path.startsWith('blob:') ? path : convertFileSrc(path));
  }, []);

  /** Fires the actual ask() call and cleans up attached images + input. */
  const executeSubmit = useCallback(
    (submitQuery: string, context: string | undefined) => {
      const readyPaths = attachedImages
        .filter((img) => img.filePath !== null)
        .map((img) => img.filePath as string);
      const images = readyPaths.length > 0 ? readyPaths : undefined;
      ask(submitQuery, context, images);
      setSelectedContext(null);
      setQuery('');
      for (const img of attachedImages) {
        URL.revokeObjectURL(img.blobUrl);
      }
      setAttachedImages([]);
      inputRef.current!.style.height = 'auto';
    },
    [ask, attachedImages, setSelectedContext],
  );

  /**
   * Async handler for the `/screen` command path. Invokes the Rust
   * `capture_full_screen_command`, which silently captures the screen
   * (excluding Thuki's own windows) and returns the saved file path.
   * On success, merges the screenshot path with any manually attached
   * images and calls ask(). On error, restores the query so no input is lost.
   */
  const handleScreenSubmit = useCallback(async () => {
    // eslint-disable-next-line no-control-regex
    const CONTROL_CHARS = /[\x00-\x08\x0b\x0c\x0e-\x1f]/g;
    const sanitized = selectedContext
      ?.replace(CONTROL_CHARS, '')
      .slice(0, quote.maxContextLength);
    const context = sanitized?.trim() ? sanitized : undefined;

    const trimmed = query.trimStart();
    const cleanQuery = trimmed.slice('/screen'.length).trimStart();

    // Snapshot display paths for the pending bubble: use resolved file paths
    // for already-processed images, blob URLs for still-processing ones.
    const existingDisplayPaths = attachedImages.map(
      (img) => img.filePath ?? img.blobUrl,
    );

    // Store the original input so handleCancel can restore it if the user
    // aborts the capture before it resolves.
    const restoredQuery = `/screen${cleanQuery ? ` ${cleanQuery}` : ''}`;
    screenCaptureInputSnapshotRef.current = { query: restoredQuery, context };

    // Immediately show the user's message in chat with a loading placeholder
    // for the screenshot. This prevents double-submit spam and gives instant
    // feedback that the capture is in progress.
    screenCapturePendingRef.current = true;
    setIsSubmitPending(true);
    setPendingUserMessage({
      id: crypto.randomUUID(),
      role: 'user',
      content: cleanQuery,
      quotedText: context,
      imagePaths: [...existingDisplayPaths, SCREEN_CAPTURE_PLACEHOLDER],
    });
    setQuery('');
    setSelectedContext(null);
    /* v8 ignore start -- inputRef always set when overlay is visible */
    if (inputRef.current) inputRef.current.style.height = 'auto';
    /* v8 ignore stop */

    let screenshotPath: string;
    try {
      screenshotPath = await invoke<string>('capture_full_screen_command');
    } catch (e) {
      screenCapturePendingRef.current = false;
      screenCaptureInputSnapshotRef.current = null;
      // Capture failed: restore input state so the user can retry or edit.
      setIsSubmitPending(false);
      setPendingUserMessage(null);
      setQuery(restoredQuery);
      setSelectedContext(context ?? null);
      // Surface the Rust error directly: the backend already provides
      // descriptive messages (permission prompts, null-image diagnostics, etc.).
      // Tauri v2 rejects with the Err(String) value as a plain string.
      setCaptureError(
        typeof e === 'string' ? e : e instanceof Error ? e.message : String(e),
      );
      return;
    }

    // Check for mid-flight cancellation before touching any state.
    // handleCancel sets screenCapturePendingRef.current = false as a signal.
    const wasCancelled = !screenCapturePendingRef.current;
    screenCapturePendingRef.current = false;
    screenCaptureInputSnapshotRef.current = null;
    if (wasCancelled) return;

    // Capture succeeded: finalize the submit.
    setCaptureError(null);
    setIsSubmitPending(false);
    setPendingUserMessage(null);

    const readyPaths = attachedImages
      .filter((img) => img.filePath !== null)
      .map((img) => img.filePath as string);
    readyPaths.push(screenshotPath);

    ask(cleanQuery, context, readyPaths);
    for (const img of attachedImages) {
      URL.revokeObjectURL(img.blobUrl);
    }
    setAttachedImages([]);
  }, [
    query,
    selectedContext,
    attachedImages,
    ask,
    setSelectedContext,
    setCaptureError,
  ]);

  const handleSubmit = useCallback(() => {
    if (
      (query.trim().length === 0 && attachedImages.length === 0) ||
      isGenerating
    )
      return;

    // Clear any stale capture error from a previous attempt.
    setCaptureError(null);

    // Detect /screen command at the very start of the message.
    const trimmedQuery = query.trimStart();
    const isScreenCommand =
      trimmedQuery.startsWith('/screen') &&
      (trimmedQuery.length === '/screen'.length ||
        trimmedQuery['/screen'.length] === ' ');

    if (isScreenCommand) {
      // Fire-and-forget: the async path handles cleanup and ask() invocation.
      void handleScreenSubmit();
      return;
    }

    // Sanitize externally-sourced context: strip control characters and enforce
    // a length cap to limit prompt-injection surface from host-app selections.
    // eslint-disable-next-line no-control-regex
    const CONTROL_CHARS = /[\x00-\x08\x0b\x0c\x0e-\x1f]/g;
    const sanitized = selectedContext
      ?.replace(CONTROL_CHARS, '')
      .slice(0, quote.maxContextLength);
    const context = sanitized?.trim() ? sanitized : undefined;

    // If all images are ready (or there are none), submit immediately.
    const hasPendingImages = attachedImages.some(
      (img) => img.filePath === null,
    );
    if (!hasPendingImages) {
      executeSubmit(query, context);
      return;
    }

    // Images are still processing — store the intent and wait. The effect
    // below will fire the actual ask() once every image has resolved.
    pendingSubmitRef.current = { query, context };
    setIsSubmitPending(true);

    // Show the user's message immediately in the chat view. Use file paths
    // for already-processed images (no loading spinner) and blob URLs only
    // for images still being processed (ChatBubble shows a spinner for blob: URLs).
    setPendingUserMessage({
      id: crypto.randomUUID(),
      role: 'user',
      content: query,
      quotedText: context,
      imagePaths: attachedImages.map((img) => img.filePath ?? img.blobUrl),
    });

    setQuery('');
    setSelectedContext(null);
    inputRef.current!.style.height = 'auto';
  }, [
    query,
    isGenerating,
    executeSubmit,
    handleScreenSubmit,
    selectedContext,
    setSelectedContext,
    attachedImages,
    setCaptureError,
  ]);

  // When a pending submit exists and all images finish processing, fire it.
  // Reads `attachedImages` directly (not via `executeSubmit` closure) to
  // guarantee the effect always sees the freshest file paths.
  /* eslint-disable @eslint-react/set-state-in-effect -- intentional: effect
     reacts to image processing completion and must synchronously transition
     state (pending → submitted) in the same tick to avoid stale renders. */
  useEffect(() => {
    if (!pendingSubmitRef.current) return;
    if (attachedImages.length === 0) {
      // All images failed — restore the user's query so their text isn't lost.
      const { query: savedQuery, context: savedContext } =
        pendingSubmitRef.current;
      pendingSubmitRef.current = null;
      setIsSubmitPending(false);
      setPendingUserMessage(null);
      setQuery(savedQuery);
      setSelectedContext(savedContext ?? null);
      return;
    }
    // Wait until every image has finished backend processing.
    const allReady = attachedImages.every((img) => img.filePath !== null);
    if (!allReady) return;

    const { query: pendingQuery, context } = pendingSubmitRef.current;
    pendingSubmitRef.current = null;
    setIsSubmitPending(false);
    // Clear the preview message — ask() will add the real one with file paths.
    setPendingUserMessage(null);

    const images = attachedImages.map((img) => img.filePath as string);
    void ask(pendingQuery, context, images);
    setSelectedContext(null);
    for (const img of attachedImages) {
      URL.revokeObjectURL(img.blobUrl);
    }
    setAttachedImages([]);
  }, [attachedImages, ask, setSelectedContext]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  /**
   * Unified cancel handler: reverts a pending submit (undo-send), clears an
   * in-flight /screen capture, or cancels an active Ollama generation.
   *
   * Three cases:
   * 1. Image-processing pending (`pendingSubmitRef.current` is set): restore
   *    query and attached images so the user can re-submit or edit.
   * 2. Screen-capture in-flight (`isSubmitPending` true but ref is null):
   *    clear pending state. The async capture may still complete on the Rust
   *    side, but `isSubmitPending` being false when the result arrives will
   *    cause `handleScreenSubmit` to attempt ask() on stale state. To prevent
   *    that, we track the abandonment via a flag so the async tail is a no-op.
   * 3. Ollama generation active: delegate to the streaming cancel.
   */
  const handleCancel = useCallback(() => {
    if (isSubmitPending && pendingSubmitRef.current) {
      // Case 1: image-processing pending. Restore input state.
      setQuery(pendingSubmitRef.current.query);
      setSelectedContext(pendingSubmitRef.current.context ?? null);
      pendingSubmitRef.current = null;
      setIsSubmitPending(false);
      setPendingUserMessage(null);
      requestAnimationFrame(() => inputRef.current?.focus());
      return;
    }
    if (isSubmitPending) {
      // Case 2: /screen capture in flight. Signal cancellation via ref so the
      // async tail in handleScreenSubmit skips ask() when capture resolves.
      // Restore the ask bar to what it looked like before the capture started.
      screenCapturePendingRef.current = false;
      const snapshot = screenCaptureInputSnapshotRef.current;
      screenCaptureInputSnapshotRef.current = null;
      setIsSubmitPending(false);
      setPendingUserMessage(null);
      /* v8 ignore start -- snapshot is always set when isSubmitPending is true via /screen */
      if (snapshot) {
        setQuery(snapshot.query);
        setSelectedContext(snapshot.context ?? null);
      }
      /* v8 ignore stop */
      requestAnimationFrame(() => inputRef.current?.focus());
      return;
    }
    cancel();
  }, [isSubmitPending, cancel, setSelectedContext]);

  /**
   * Synchronizes the React animation state with Tauri-driven overlay visibility
   * requests emitted from the Rust backend.
   */
  useEffect(() => {
    let unlistenVisibility: (() => void) | undefined;
    let unlistenOnboarding: (() => void) | undefined;

    const attachListeners = async () => {
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
      unlistenOnboarding = await listen<void>(ONBOARDING_EVENT, () => {
        setShowOnboarding(true);
      });
      // Both listeners registered — safe to let Rust decide what to show on launch.
      await invoke('notify_frontend_ready');
    };

    void attachListeners();
    return () => {
      unlistenVisibility?.();
      unlistenOnboarding?.();
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

  if (showOnboarding) {
    return <OnboardingView />;
  }

  return (
    // Minimal padding (pt-2 pb-6) provides just enough physical clearance for the
    // tightened drop shadow to render without clipping at the native window edge.
    <div
      ref={outerContainerRef}
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
                className={`morphing-container relative flex flex-col bg-surface-base backdrop-blur-2xl border border-surface-border max-h-[600px] overflow-hidden ${
                  isChatMode
                    ? 'rounded-lg shadow-chat'
                    : 'rounded-2xl shadow-bar'
                }`}
              >
                {/* Chat Messages Area — morphs in when in chat mode */}
                <AnimatePresence>
                  {isChatMode ? (
                    <ConversationView
                      messages={
                        pendingUserMessage
                          ? [...messages, pendingUserMessage]
                          : messages
                      }
                      streamingContent={streamingContent}
                      isGenerating={isGenerating || isSubmitPending}
                      onClose={handleCloseOverlay}
                      onSave={handleSave}
                      isSaved={isSaved}
                      canSave={canSave}
                      onNewConversation={handleNewConversation}
                      onHistoryOpen={handleHistoryToggle}
                      onImagePreview={handleChatImagePreview}
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
                          height: {
                            duration: 0.3,
                            ease: [0.33, 1, 0.68, 1],
                          },
                          opacity: { duration: 0.2, delay: 0.08 },
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

                {/* Capture error banner: shown when /screen capture fails so
                    the user knows why the message was not sent. */}
                {captureError && (
                  <div className="px-4 py-2 border-t border-red-900/30">
                    <p className="text-red-400 text-xs leading-relaxed">
                      {captureError}
                    </p>
                  </div>
                )}

                {/* Input Bar — always pinned to the bottom */}
                <AskBarView
                  query={query}
                  setQuery={setQuery}
                  isChatMode={isChatMode}
                  isGenerating={isGenerating}
                  isSubmitPending={isSubmitPending}
                  onSubmit={handleSubmit}
                  onCancel={handleCancel}
                  inputRef={inputRef}
                  selectedText={selectedContext ?? undefined}
                  onHistoryOpen={handleHistoryToggle}
                  attachedImages={isSubmitPending ? [] : attachedImages}
                  onImagesAttached={handleImagesAttached}
                  onImageRemove={handleImageRemove}
                  onImagePreview={handleAskBarImagePreview}
                  onScreenshot={handleScreenshot}
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
                      showNewConversation={false}
                      pendingNewConversation={pendingNewConversation}
                      onSaveAndNew={handleSaveAndNew}
                      onJustNew={handleJustNew}
                      onCancelNew={() => setIsHistoryOpen(false)}
                    />
                  </motion.div>
                ) : null}
              </AnimatePresence>
            </div>
          </motion.div>
        ) : null}
      </AnimatePresence>
      <ImagePreviewModal
        imageUrl={previewImageUrl}
        onClose={() => setPreviewImageUrl(null)}
      />
    </div>
  );
}

export default App;
