import { motion, AnimatePresence } from 'framer-motion';
import type React from 'react';
import { createPortal, flushSync } from 'react-dom';
import {
  useState,
  useEffect,
  useCallback,
  useMemo,
  useRef,
  useLayoutEffect,
} from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import {
  getCurrentWindow,
  currentMonitor,
  availableMonitors,
} from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { useOllama } from './hooks/useOllama';
import type { Message } from './hooks/useOllama';
import { useConversationHistory } from './hooks/useConversationHistory';
import { useModelSelection } from './hooks/useModelSelection';
import { useModelCapabilities } from './hooks/useModelCapabilities';
import {
  getCapabilityConflict,
  getEnvironmentMessage,
  isComposeCapabilityConflict,
} from './utils/capabilityConflicts';
import {
  computeExpandTarget,
  computeCollapseTarget,
  anchorToTransformOrigin,
  pickMonitorForPoint,
  type MorphAnchor,
} from './utils/morphGeometry';
import { ConversationView } from './view/ConversationView';
import { AskBarView } from './view/AskBarView';
import { OnboardingView } from './view/onboarding/index';
import type { OnboardingStage } from './view/onboarding/index';
import { MinimizedIcon } from './components/MinimizedIcon';
import { HistoryPanel } from './components/HistoryPanel';
import { ModelPickerPanel } from './components/ModelPickerPanel';
import { ImagePreviewModal } from './components/ImagePreviewModal';
import { TipBar } from './components/TipBar';
import { UpdateFooterBar } from './components/UpdateFooterBar';
import { useTips } from './hooks/useTips';
import { useUpdater } from './hooks/useUpdater';
import type { AttachedImage } from './types/image';
import { MAX_IMAGE_SIZE_BYTES } from './types/image';
import { useConfig } from './contexts/ConfigContext';
import {
  COMMANDS,
  SCREEN_CAPTURE_PLACEHOLDER,
  buildPrompt,
} from './config/commands';
import { save as saveDialog } from '@tauri-apps/plugin-dialog';
import {
  defaultExportFilename,
  serializeForClipboard,
  serializeForFile,
} from './lib/exportSerializer';
import './App.css';

const OVERLAY_VISIBILITY_EVENT = 'thuki://visibility';

// eslint-disable-next-line no-control-regex
const CONTROL_CHARS = /[\x00-\x08\x0b\x0c\x0e-\x1f]/g;
const ONBOARDING_EVENT = 'thuki://onboarding';

/**
 * Strips control characters and enforces a length cap on externally-sourced
 * context (host-app text selections piped through the quote bar). Returns
 * undefined when the trimmed result is empty so callers can treat "no
 * context" uniformly.
 */
function sanitizeContext(
  selectedContext: string | null,
  maxContextLength: number,
): string | undefined {
  const sanitized = selectedContext
    ?.replace(CONTROL_CHARS, '')
    .slice(0, maxContextLength);
  return sanitized?.trim() ? sanitized : undefined;
}

/**
 * Builds the placeholder Message shown in the conversation while a submit is
 * deferred (image still processing or /screen capture in flight). Each
 * attached image renders as its resolved file path when available, falling
 * back to a blob URL so the bubble still shows a thumbnail with a spinner.
 * Adds a SCREEN_CAPTURE_PLACEHOLDER tile when /screen will run.
 */
function buildPendingBubble(params: {
  query: string;
  context: string | undefined;
  attachedImages: AttachedImage[];
  hasScreen: boolean;
}): Message {
  const displayPaths = params.attachedImages.map(
    (img) => img.filePath ?? img.blobUrl,
  );
  const placeholders = params.hasScreen
    ? [...displayPaths, SCREEN_CAPTURE_PLACEHOLDER]
    : displayPaths;
  return {
    id: crypto.randomUUID(),
    role: 'user',
    content: params.query,
    quotedText: params.context,
    /* v8 ignore start -- callers always have at least one image source
       (a resolved/pending image, /screen, or both); empty branch defensive */
    imagePaths: placeholders.length > 0 ? placeholders : undefined,
    /* v8 ignore stop */
  };
}

/**
 * Filters attached images down to those whose backend processing has
 * resolved (filePath !== null) and appends an optional fresh screenshot
 * path. Returned list is the source of truth for what gets sent to the
 * model or to OCR.
 */
function resolveReadyPaths(
  attachedImages: AttachedImage[],
  screenshotPath?: string,
): string[] {
  const paths = attachedImages
    .filter((img) => img.filePath !== null)
    .map((img) => img.filePath as string);
  if (screenshotPath) paths.push(screenshotPath);
  return paths;
}

/**
 * Submit intents that can be deferred while attached images finish
 * processing. The `useEffect` watching `attachedImages` switches on
 * `kind` to dispatch to the right stage-2 handler once every image
 * has a resolved `filePath`.
 *
 * Every variant carries `query` and `context` so the cancel-restore
 * path can read them uniformly.
 */
type PendingSubmit =
  | {
      kind: 'plain';
      query: string;
      context: string | undefined;
      think: boolean;
    }
  | {
      kind: 'utility-ocr';
      query: string;
      context: string | undefined;
      think: boolean;
      trigger: string;
      strippedMessage: string;
      hasScreen: boolean;
    }
  | {
      kind: 'extract';
      query: string;
      context: string | undefined;
      hasScreen: boolean;
    }
  | {
      kind: 'screen';
      query: string;
      context: string | undefined;
      think: boolean;
    };

/** Total transparent padding around the morphing container: pt-2(8) + pb-6(24) + motion py-2(16). */
const CONTAINER_VERTICAL_PADDING = 48;

/**
 * Collapsed-bar height used as the seed for the show-time upward-grow Y math
 * and as the fallback when the morphing container reports `offsetHeight === 0`.
 * Baked in: only observable for a single frame at show time before the
 * ResizeObserver replaces it with the real measured height, so a user-tunable
 * knob would have no perceptible effect.
 */
const COLLAPSED_WINDOW_HEIGHT = 80;

/**
 * Logical-pixel side length of the minimized floating-icon window. The native
 * window shrinks to this square. The 48px mascot logo is centered inside it
 * with a margin, so the working "jelly wobble" / completion pop can overshoot
 * the logo's bounds, and the status jewel's glow can bloom at the bottom-right
 * corner, without being clipped by the window frame (body overflow is hidden).
 * This size is the icon footprint fed to the edge-aware morph geometry, so
 * both the native window and the in-chat morph mascot use it.
 */
const MINIMIZED_WINDOW_SIZE = 68;

/**
 * Single source of truth for the chat-card collapse/expand tween duration,
 * in seconds. The OS window itself does NOT animate during the morph; it
 * only snap-resizes at the endpoints (`durationMs:0`) once the painted
 * content already matches. Floating-window apps (Raycast, Alfred, Spotlight,
 * Linear Cmd+K) sit at 0.15s to 0.25s for a plain close. Thuki's case is
 * different: the chat morphs into a persistent mascot rather than just
 * vanishing, so the transition needs a touch more time and travel to read as
 * one connected morph instead of a fade followed by a separate pop-in.
 *
 * This value drives the EXPAND (restore) direction only. The collapse
 * direction uses its own, longer duration (see `COLLAPSE_MORPH_DURATION_S`).
 */
const MORPH_DURATION_S = 0.36;
/**
 * Duration of the COLLAPSE (minimize) chat-card shrink+fade, in seconds.
 * Deliberately longer than the expand duration so the chat dissolving into
 * the mascot reads as a slow, cinematic morph rather than a quick vanish.
 * Only the collapse uses this; expand keeps `MORPH_DURATION_S`.
 */
const COLLAPSE_MORPH_DURATION_S = 0.55;
/**
 * Apple-style ease-out curve (matches SwiftUI `.easeOut`). Applied to the
 * chat-card collapse/expand transform so the shrink+fade reads as a clean
 * decelerating settle rather than the overshoot-and-decay shape of the
 * earlier curve.
 */
const MORPH_EASE = [0.32, 0.72, 0, 1] as const;
/**
 * Grace period added on top of a morph's animation duration before the
 * watchdog force-settles `morphPhase`, in milliseconds. The morph normally
 * settles precisely via Framer Motion's `onAnimationComplete`, but that
 * callback can fail to fire (e.g. an identical start/end target produces no
 * animation, or WKWebView throttles the rAF clock on the nonactivating
 * panel). Without a settle the machine strands mid-morph and the mascot can
 * end up animated to invisible. The watchdog guarantees the phase always
 * reaches its terminal state (`minimized` or `idle`) regardless.
 */
const MORPH_SETTLE_GRACE_MS = 250;

/**
 * Authoritative deadline from the start of the hide transition to the native
 * window hide call. Accounts for WKWebView `requestAnimationFrame` throttling
 * in non-key windows, which stalls spring animations indefinitely and makes
 * `AnimatePresence.onExitComplete` unreliable when the panel is unfocused.
 * Baked in: subjectively imperceptible across the usable range, and lowering
 * it below the exit-animation duration causes a visible pop.
 */
const HIDE_COMMIT_DELAY_MS = 350;

/**
 * Parses a message to detect all valid slash commands present as whole words.
 * Derives detectable commands from the COMMANDS registry so adding a command
 * to the registry is sufficient (no hardcoded trigger strings here).
 * Also returns the message with command triggers stripped for the LLM.
 */
export function parseCommands(text: string): {
  found: Set<string>;
  strippedMessage: string;
} {
  const words = text.trim().split(/\s+/);
  const triggerSet = new Set(COMMANDS.map((c) => c.trigger));
  const found = new Set<string>();
  const remaining: string[] = [];
  for (const word of words) {
    if (triggerSet.has(word)) {
      found.add(word);
    } else {
      remaining.push(word);
    }
  }
  return { found, strippedMessage: remaining.join(' ') };
}

type OverlayVisibilityPayload =
  | {
      state: 'show';
      selected_text: string | null;
      window_x: number | null;
      window_y: number | null;
      screen_bottom_y: number | null;
    }
  | { state: 'hide-request' }
  | { state: 'restore' };
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
   * Minimize/restore morph state machine. The morph happens entirely in the
   * web layer (GPU transforms); the OS window only snap-resizes at the
   * endpoints (durationMs:0) when the painted content already matches the
   * target size, so the resize is invisible against the transparent NSPanel.
   *
   * - `idle`:       normal chat/ask. Content-driven window sizing (ResizeObserver
   *                  + the two chat useLayoutEffects). Byte-identical to before.
   * - `collapsing`: OS window stays at full chat size. The chat card scales
   *                  down + translates toward the top-left while the bare
   *                  mascot crossfades in. On complete: snap the OS window to
   *                  48x48 (content is already the mascot, so it is invisible).
   * - `minimized`:  settled. OS window is 48x48 so clicks pass through to
   *                  the desktop around it. Only `<MinimizedIcon>` renders.
   * - `expanding`:  OS window resized back to full chat size on the SAME tick
   *                  the in-page expand starts (no await). Mascot scales out
   *                  into the chat card. On complete: back to `idle`.
   */
  const [morphPhase, setMorphPhase] = useState<
    'idle' | 'collapsing' | 'minimized' | 'expanding'
  >('idle');
  /**
   * Which corner the chat is pinned to during the minimize/restore morph.
   * Chosen edge-aware at expand time (so the chat unfolds into open space when
   * the icon is near a screen edge) and reused on the matching collapse so the
   * icon returns to the same spot. Drives both the chat card's
   * transform-origin and where the floating mascot is rendered, which is what
   * keeps the icon visually stationary while the window resizes under it.
   * Defaults to top-left and is reset to top-left whenever the overlay is
   * shown fresh.
   */
  const [morphAnchor, setMorphAnchor] = useState<MorphAnchor>('tl');
  /**
   * Gates the chat-card's grow animation during expand. While false, the chat
   * is held collapsed (scale 0.34, opacity 0 — invisible) so it cannot be seen
   * during the native window move that repositions the overlay on screen.
   * Flipped true only AFTER that move (and a paint yield), so the chat grows
   * out of the anchor corner in the already-correctly-positioned window. This
   * is what keeps the move flicker-free: across the move, BOTH the mascot
   * (opacity 0) and the chat (collapsed) are invisible, so the ~1 stale frame
   * WebKit may displace to the new origin contains nothing.
   */
  const [expandReady, setExpandReady] = useState(false);
  /**
   * True whenever the overlay is not in the normal chat/ask state, i.e.
   * during the collapse/expand morph or while settled as the floating icon.
   * Gates the chat sizing machinery (ResizeObserver + chat useLayoutEffects)
   * off so transforms are never fought by layout writes, and keeps
   * Esc/Cmd+W ignored across the whole minimized lifecycle.
   */
  const isMinimized = morphPhase !== 'idle';
  /** True only in the settled floating-icon state (chat subtree unmounted). */
  const isSettledMinimized = morphPhase === 'minimized';
  /** True only while a collapse/expand transform morph is in flight. */
  const isMorphing = morphPhase === 'collapsing' || morphPhase === 'expanding';
  /** True when a streaming completion finished while the overlay was minimized. */
  const [unseenCompletion, setUnseenCompletion] = useState(false);
  /** Non-null when the backend signals onboarding is needed; holds the current stage. */
  const [onboardingStage, setOnboardingStage] =
    useState<OnboardingStage | null>(null);

  /**
   * Whether the ask-bar history panel is currently open.
   * Distinct from the chat-mode history dropdown (controlled by the same toggle
   * but rendered differently based on `isChatMode`).
   */
  const [isHistoryOpen, setIsHistoryOpen] = useState(false);

  /** Whether the model picker panel is currently open. Mutually exclusive with `isHistoryOpen`. */
  const [isModelPickerOpen, setIsModelPickerOpen] = useState(false);
  /** Whether the chat-header export popover (clipboard / file) is currently open. */
  const [isExportOpen, setIsExportOpen] = useState(false);
  /**
   * Ref to the export popover root. Used by the outside-click effect to
   * keep the popover open while the user is clicking inside it.
   */
  const exportPopoverRef = useRef<HTMLDivElement>(null);
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
    activeModel,
    availableModels,
    ollamaReachable,
    refreshModels,
    setActiveModel,
  } = useModelSelection();

  const { capabilities: modelCapabilities, refresh: refreshModelCapabilities } =
    useModelCapabilities();

  /** Capability flags for the currently active model, or undefined if not loaded yet. */
  const activeModelCapabilities = activeModel
    ? modelCapabilities[activeModel]
    : undefined;

  /**
   * Pulses true to trigger the ask-bar shake animation when the
   * submit-time gate refuses a message, then resets so the next blocked
   * submit gets its own animation. Reset is set just over the 500 ms
   * keyframe duration in `AskBarView` so the bar never snaps back
   * mid-animation if React schedules the state flip on the exact frame
   * Framer is finishing.
   */
  const [shakeAskBar, setShakeAskBar] = useState(false);
  useEffect(() => {
    if (!shakeAskBar) return;
    const timer = setTimeout(() => setShakeAskBar(false), 600);
    return () => clearTimeout(timer);
  }, [shakeAskBar]);

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
    ask,
    askSearch,
    cancel,
    isGenerating,
    searchStage,
    reset,
    loadMessages,
    getTraceConversationId,
    addOcrTurn,
  } = useOllama(activeModel, handleTurnComplete);

  /**
   * Sticky flag: once the user invokes `/search`, subsequent submits in the
   * same conversation route through the search pipeline automatically until
   * the pipeline delivers a final answer (or the conversation is reset/loaded
   * /closed). The backend LLM classifies each turn and decides whether to
   * clarify, answer from context, or perform a fresh web search.
   */
  const [searchActive, setSearchActive] = useState(false);

  const inputRef = useRef<HTMLTextAreaElement>(null);

  /** Images attached to the current (unsent) message. Blob URLs render
   *  immediately; file paths are set asynchronously after Rust processing. */
  const [attachedImages, setAttachedImages] = useState<AttachedImage[]>([]);
  /** URL of the image currently open in the preview modal (blob or asset URL). */
  const [previewImageUrl, setPreviewImageUrl] = useState<string | null>(null);
  /**
   * Drag state passed to AskBarView for visual ring feedback.
   * "normal" = under capacity (violet ring); "max" = at capacity (red ring + label).
   * null = no active drag.
   */
  const [isDragOver, setIsDragOver] = useState<'normal' | 'max' | null>(null);

  /** When the user submits while images are still processing, the submit
   *  intent is stored here. The effect below watches `attachedImages` and
   *  dispatches the matching stage-2 handler once every image has a
   *  resolved `filePath`. The discriminated `kind` tells the resolver
   *  which handler to run. */
  const pendingSubmitRef = useRef<PendingSubmit | null>(null);
  /** True while waiting for images to finish processing before a deferred
   *  submit. Drives the "waiting" UI state in the ask bar. */
  const [isSubmitPending, setIsSubmitPending] = useState(false);
  /** Error message from a failed /screen capture or any other gate that
   *  surfaces user-facing feedback. Shown inline above the ask bar so the
   *  user knows the submission did not go through. Auto-clears after a
   *  short linger so a one-off mistake does not leave the banner up
   *  forever; the next submit also clears it preemptively. */
  const [captureError, setCaptureError] = useState<string | null>(null);
  /**
   * Auto-dismiss the capture-error banner after a short linger so a
   * one-off mistake (empty `/extract`, empty `/export`, OCR miss, etc.)
   * does not leave a red banner up indefinitely. Mirrors the
   * `shakeAskBar` self-clearing pattern.
   *
   * 5 seconds reads as a deliberate auto-hide rather than a flash and
   * gives the user time to read a one-line message twice. The banner is
   * also cleared at the top of `handleSubmit` so a fresh submit attempt
   * always starts clean regardless of timing.
   */
  useEffect(() => {
    if (!captureError) return;
    const timer = setTimeout(() => setCaptureError(null), 5000);
    return () => clearTimeout(timer);
  }, [captureError]);
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
   * Session counter - incremented on each overlay open. Used in the motion
   * key to force AnimatePresence to fully unmount the stale tree before
   * mounting a fresh one, preventing a flash of the previous conversation.
   */
  const [sessionId, setSessionId] = useState(0);
  const [selectedContext, setSelectedContext] = useState<string | null>(null);
  const config = useConfig();
  const quote = config.quote;

  /**
   * True when the window is near the screen bottom and should grow upward.
   * Flips the outer container to `justify-end` so content pins to the bottom.
   */
  const [growsUpward, setGrowsUpward] = useState(false);

  /**
   * Determines whether the UI has entered "chat mode" - i.e., the morphing
   * chat window state with message bubbles. Transitions from input-bar mode
   * to chat-window mode are animated via Framer Motion `layout` prop.
   */
  const isChatMode = messages.length > 0 || isGenerating || isSubmitPending;
  const previousIsChatModeRef = useRef(isChatMode);

  /**
   * The bookmark save button is active once the AI has produced at least one
   * complete response. We check for an assistant message rather than any message
   * so the button never appears during the very first user-only half-turn.
   */
  const canSave = !isGenerating && messages.some((m) => m.role === 'assistant');
  const shouldRenderOverlay = overlayState === 'visible';
  const {
    tip: activeTip,
    tipKey,
    isVisible: isTipVisible,
  } = useTips(shouldRenderOverlay);

  // Animate the tip typewriter only the FIRST time a given tip (tipKey) is
  // shown. Minimizing unmounts the chat subtree (and TipBar with it); on
  // restore TipBar remounts with the SAME tipKey, which would otherwise replay
  // the typewriter from scratch. We remember the last tipKey we let animate
  // (this ref survives minimize because App itself never unmounts) and tell
  // TipBar to render an already-seen tip as static text instead of re-typing.
  const animatedTipKeyRef = useRef(-1);
  useEffect(() => {
    // Once a tip is on screen, mark its key as animated. This effect runs
    // after render, so the render that first shows a new tip still sees the
    // old key and animates; only later mounts of the same key are static.
    if (isTipVisible) {
      animatedTipKeyRef.current = tipKey;
    }
  }, [isTipVisible, tipKey]);
  const tipAlreadyAnimated = tipKey === animatedTipKeyRef.current;

  const updater = useUpdater();
  const chatSnoozed = useMemo(
    () => (updater.state.chat_snoozed_until ?? 0) * 1000 > Date.now(),
    [updater.state.chat_snoozed_until],
  );
  const showUpdate = !!updater.state.update && !chatSnoozed;

  /**
   * Reference stored for ResizeObserver cleanup.
   */
  const observerRef = useRef<ResizeObserver | null>(null);

  /**
   * The layout-wrapper DOM node the ResizeObserver watches. Stored so the
   * restore path can force a fresh observation once the expand morph settles
   * (the observer does not re-fire on its own then, because the wrapper's
   * layout box is unchanged across the morph — only its CSS transform is).
   */
  const layoutWrapperNodeRef = useRef<HTMLDivElement | null>(null);

  /**
   * Pending watchdog timer id that force-settles `morphPhase` if the morph's
   * `onAnimationComplete` never fires. 0 means no timer scheduled
   * (`clearTimeout(0)` is a safe no-op).
   */
  const morphSettleTimerRef = useRef<number>(0);

  /**
   * Mirror of `growsUpward` as a ref so the ResizeObserver closure can read
   * it without being recreated on each state change.
   */
  const growsUpwardRef = useRef(false);

  /**
   * Stores the window's fixed bottom Y and X for upward-growth sessions.
   * The bottom stays pinned while the top edge moves up as content grows.
   */
  const windowPosRef = useRef({ x: 0, bottomY: 0 });

  /**
   * Mirror of `isGenerating` as a ref so the ResizeObserver closure can
   * check streaming state without being recreated on each render.
   */
  const isGeneratingRef = useRef(false);
  isGeneratingRef.current = isGenerating;

  /**
   * Mirror of `isMinimized` as a ref so the ResizeObserver closure can skip
   * chat sizing across the entire minimized lifecycle (collapsing → settled →
   * expanding). True whenever `morphPhase !== 'idle'`.
   */
  const isMinimizedRef = useRef(false);
  isMinimizedRef.current = isMinimized;

  /**
   * True only while a collapse/expand transform morph is in flight. Read by
   * the ResizeObserver and the two chat `useLayoutEffect`s to no-op so the
   * in-page GPU transform is never fought by a width/height/min-height layout
   * write (which would reflow the heavy chat tree and blank the morph).
   * Mirrored from the `isMorphing` derived state every render.
   */
  const isMorphingRef = useRef(false);
  isMorphingRef.current = isMorphing;

  /**
   * Mirror of `morphPhase` as a ref so the transform wrapper's
   * `onAnimationComplete` handler reads the live phase instead of a stale
   * closure value (the handler is a `useCallback` captured before the state
   * update that schedules the animation completion).
   */
  const morphPhaseRef = useRef(morphPhase);
  morphPhaseRef.current = morphPhase;

  /** Mirror of `morphAnchor` for reading inside callbacks (collapse snap). */
  const morphAnchorRef = useRef(morphAnchor);
  morphAnchorRef.current = morphAnchor;

  /**
   * High-water mark for window height during streaming. While the LLM is
   * generating, the window only grows (never shrinks) to prevent jitter
   * from Streamdown's block-element reflows. Reset when generation ends
   * or a new session starts.
   */
  const maxHeightRef = useRef(0);

  /**
   * Mirrors of the user-tunable window dimensions from `[window]` config.
   * Stored in refs so the ResizeObserver closure can read the latest value
   * without being recreated on each config edit (which would tear down /
   * recreate the observer mid-stream). The effect below keeps refs in sync
   * with React state and proactively re-applies width edits via `setSize`
   * (the ResizeObserver only fires on DOM height changes, so a pure width
   * change would otherwise stay invisible until the next content reflow).
   */
  const overlayWidthRef = useRef(config.window.overlayWidth);
  const maxChatHeightRef = useRef(config.window.maxChatHeight);
  useEffect(() => {
    overlayWidthRef.current = config.window.overlayWidth;
    maxChatHeightRef.current = config.window.maxChatHeight;
    /* v8 ignore start -- requires real Tauri webview to setSize */
    if (overlayState === 'visible') {
      const node = morphingContainerNodeRef.current;
      const currentHeight = node
        ? Math.ceil(node.getBoundingClientRect().height) +
          CONTAINER_VERTICAL_PADDING
        : COLLAPSED_WINDOW_HEIGHT;
      void getCurrentWindow().setSize(
        new LogicalSize(config.window.overlayWidth, currentHeight),
      );
    }
    /* v8 ignore stop */
  }, [config.window.overlayWidth, config.window.maxChatHeight, overlayState]);

  /**
   * Drives the typography CSS variables on `<html>` from the user-tunable
   * `[window]` typography knobs. Consumers (the AI markdown body, the user
   * chat bubble text, and the AskBar textarea + caret-tracking mirror) read
   * the variables directly; this is the single write path that resyncs
   * after every Settings save (via the `thuki://config-updated` refresh in
   * `ConfigContext`).
   */
  useEffect(() => {
    const root = document.documentElement;
    root.style.setProperty(
      '--thuki-text-base',
      `${config.window.textBasePx}px`,
    );
    root.style.setProperty(
      '--thuki-text-line-height',
      `${config.window.textLineHeight}`,
    );
    root.style.setProperty(
      '--thuki-text-letter-spacing',
      `${config.window.textLetterSpacingPx}px`,
    );
    root.style.setProperty(
      '--thuki-text-font-weight',
      `${config.window.textFontWeight}`,
    );
  }, [
    config.window.textBasePx,
    config.window.textLineHeight,
    config.window.textLetterSpacingPx,
    config.window.textFontWeight,
  ]);

  /**
   * Callback ref to reliably attach the ResizeObserver when the conditionally
   * rendered Framer Motion container actually mounts in the DOM. This fixes
   * the bug where a standard useEffect would run before the DOM node was ready,
   * leaving the native window stuck at 600x700.
   *
   * When `growsUpwardRef` is true (window near screen bottom), the observer
   * also repositions the window upward to keep its bottom pinned as the
   * conversation grows.
   */
  const setContainerRef = useCallback((node: HTMLDivElement | null) => {
    morphingContainerNodeRef.current = node;
  }, []);

  /**
   * Callback ref for the layout wrapper that surrounds both the morphing
   * container and the footer slot. Attaches the ResizeObserver so the native
   * window tracks the combined height: container + footer. The footer sits
   * outside the inner overflow-hidden container so it is never clipped when
   * chat is at max height, and the wrapper's natural height grows to include it.
   */
  const setLayoutWrapperRef = useCallback((node: HTMLDivElement | null) => {
    layoutWrapperNodeRef.current = node;
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

              // While morphing or minimized the native `animate_overlay_frame`
              // command owns the window frame (Core Animation drives the
              // tween; the icon square is held by the collapse target). The
              // observer must not call setSize/set_window_frame here or it
              // would fight the native animation, so do nothing.
              if (isMorphingRef.current || isMinimizedRef.current) {
                continue;
              }

              // Settled chat/ask. Unchanged content-driven sizing.
              // Total vertical room: 8px (pt-2) + 24px (pb-6) + 16px (motion py-2) = 48px.
              // This ensures the tightened drop shadows aren't clipped by the native window edge.
              let targetHeight =
                Math.ceil(rect.height) + CONTAINER_VERTICAL_PADDING;

              // During streaming, only allow the window to grow (never
              // shrink) to prevent jitter from Streamdown block reflows.
              if (isGeneratingRef.current) {
                if (targetHeight > maxHeightRef.current) {
                  maxHeightRef.current = targetHeight;
                } else {
                  targetHeight = maxHeightRef.current;
                }
              }

              if (growsUpwardRef.current) {
                // Grow upward: pin the window bottom and expand the top edge.
                // Clamp Y so the window never extends above the menu bar.
                const { x, bottomY } = windowPosRef.current;
                const newY = Math.max(0, bottomY - targetHeight);
                void invoke('set_window_frame', {
                  x,
                  y: newY,
                  width: overlayWidthRef.current,
                  height: targetHeight,
                });
              } else {
                void getCurrentWindow().setSize(
                  new LogicalSize(overlayWidthRef.current, targetHeight),
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
   * Reset the high-water mark when streaming finishes so the window can
   * shrink back to its natural content height on the next resize event.
   */
  useEffect(() => {
    if (!isGenerating) {
      maxHeightRef.current = 0;
    }
  }, [isGenerating]);

  /* eslint-disable @eslint-react/set-state-in-effect -- intentional: close
     the picker when the user triggers generation so it can't stay open over
     a streaming response. No secondary effects are triggered by this reset. */
  useEffect(() => {
    if (isGenerating || isSubmitPending) {
      setIsModelPickerOpen(false);
    }
  }, [isGenerating, isSubmitPending]);
  /* eslint-enable @eslint-react/set-state-in-effect */

  /**
   * Replays the entrance sequence by transitioning the overlay to the visible state.
   * Clears conversation state for a fresh session each time the overlay appears.
   */
  const replayEntranceAnimation = useCallback(
    (
      context: string | null,
      windowX: number | null,
      windowY: number | null,
      screenBottomY: number | null,
    ) => {
      // A fresh show always starts from the normal chat/ask state. Reset any
      // morph phase (and cancel a pending morph watchdog) left over from a
      // prior minimized session that was hidden without settling, so the
      // overlay never reappears stuck as the icon or mid-morph. Also clear the
      // unseen-completion indicator since this is a brand-new session.
      clearTimeout(morphSettleTimerRef.current);
      morphSettleTimerRef.current = 0;
      setMorphPhase('idle');
      setUnseenCompletion(false);
      // Grow-up geometry mirrored in handleRestore; keep both in sync.
      const shouldGrowUp =
        windowY !== null &&
        screenBottomY !== null &&
        windowY + maxChatHeightRef.current + CONTAINER_VERTICAL_PADDING >
          screenBottomY;
      growsUpwardRef.current = shouldGrowUp;
      setGrowsUpward(shouldGrowUp);
      maxHeightRef.current = 0;
      // A freshly shown overlay has no prior expand to mirror, so reset the
      // morph anchor to the top-left default; the next minimize folds the icon
      // into the chat's top-left, and a later expand recomputes edge-awareness
      // from the icon's dragged position.
      setMorphAnchor('tl');
      morphAnchorRef.current = 'tl';
      setExpandReady(false);
      if (shouldGrowUp && windowX !== null && windowY !== null) {
        windowPosRef.current = {
          x: windowX,
          bottomY: windowY + COLLAPSED_WINDOW_HEIGHT,
        };
      }
      setSessionId((id) => id + 1);
      setQuery('');
      setSelectedContext(context);
      setIsHistoryOpen(false);
      setIsModelPickerOpen(false);
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
      setSearchActive(false);

      void refreshModels();
      reset();
      resetHistory();
      setOverlayState('visible');
    },
    [reset, resetHistory, refreshModels],
  );

  /**
   * Moves the overlay into an exit phase. The actual Tauri window hide call is
   * deferred until Framer Motion finishes the exit transition.
   */
  const requestHideOverlay = useCallback(() => {
    void cancel();
    growsUpwardRef.current = false;
    setGrowsUpward(false);
    screenCapturePendingRef.current = false;
    screenCaptureInputSnapshotRef.current = null;
    setSearchActive(false);
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
  }, [cancel]);

  /**
   * Restores the parked conversation. The OS window is snapped back to full
   * chat size (`durationMs:0`, instant + invisible against the transparent
   * NSPanel) on the SAME tick the in-page expand transform starts; never
   * awaited first, since awaiting would briefly show the 48px mascot inside a
   * full-size click-capturing rect. The mascot scales out into the chat card;
   * the expand wrapper's `onAnimationComplete` hands back to `idle`, where the
   * ResizeObserver resumes content-driven sizing and fine-tunes the height.
   *
   * Upward-growth geometry is recomputed from the live Tauri window position
   * so the restored chat anchors correctly. The recompute is wrapped so a
   * geometry-query failure still completes the restore (defaults to no
   * grow-up) instead of leaving the overlay stuck minimized.
   */
  const handleRestore = useCallback(() => {
    // Only ever expand from a settled `minimized` state. A restore request
    // from any other phase (a stale Rust `restore` event after rapid
    // toggling, or a React/Rust minimized-flag desync) must NOT start an
    // expand morph: the `expanding` transform target is identical to the
    // `idle` target ({scale:1,opacity:1}), so Framer Motion runs no animation
    // and `onAnimationComplete` never fires. That would strand `morphPhase`
    // in `expanding` forever, where the mascot animates itself to opacity 0
    // and stays there: the "icon disappears after many toggles" bug. Re-sync
    // the Rust minimized flag (so the two sides agree again) and bail.
    if (morphPhaseRef.current !== 'minimized') {
      void invoke('set_overlay_minimized', { minimized: false });
      return;
    }
    // Keep the panel key so WKWebView does not throttle the expand
    // animation's requestAnimationFrame clock (mirrors handleMinimize).
    // Otherwise `onAnimationComplete` can stall and the phase never settles
    // back to `idle`.
    void getCurrentWindow().setFocus();
    setUnseenCompletion(false);
    setMorphPhase('expanding');
    // Hold the chat collapsed (invisible) until the window has moved; see
    // expandReady. Combined with the mascot being opacity 0 during expand,
    // this means nothing is visible while the native window repositions.
    setExpandReady(false);
    void invoke('set_overlay_minimized', { minimized: false });
    // The window currently sits where the floating icon is (the user may have
    // dragged it anywhere), so we recompute the anchor live from the icon's
    // current position. computeExpandTarget picks which corner of the panel is
    // pinned to the icon (so it unfolds into open space) and the resulting
    // on-screen top-left. That anchor drives the chat's transform-origin and
    // the mascot's corner. Height includes CONTAINER_VERTICAL_PADDING so the
    // bottom composer is not clipped before settleMorphPhase's re-measure.
    //
    // Flicker-free ordering (all three layers matter):
    //   1. the mascot is opacity 0 the moment morphPhase is 'expanding';
    //   2. the chat is held collapsed (expandReady=false) so it too is
    //      invisible;
    //   3. we then YIELD for a paint (double rAF) so WebKit actually paints
    //      that all-invisible state before the native window move — otherwise
    //      WebKit's last painted frame (the still-visible mascot from before
    //      the click) is what gets displaced to the new window origin for ~1
    //      frame, which is the jump. Only after the move do we release the
    //      chat to grow (expandReady=true); it starts at opacity 0, so even a
    //      stale post-move frame shows nothing.
    const fullWidth = overlayWidthRef.current;
    const fullHeight = maxChatHeightRef.current + CONTAINER_VERTICAL_PADDING;
    const yieldForPaint = () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
      });
    void (async () => {
      try {
        const win = getCurrentWindow();
        const [pos, scale, currentMon] = await Promise.all([
          win.outerPosition(),
          win.scaleFactor(),
          currentMonitor(),
        ]);
        const iconX = pos.x / scale;
        const iconY = pos.y / scale;
        // Logical bounds of the monitor under the icon. `currentMonitor()` can
        // return null transiently during a display-topology change; rather
        // than drop edge-awareness entirely (which can let the chat expand off
        // an edge), recover by scanning `availableMonitors()` for the display
        // actually under the icon. Containment matching can never pick a wrong
        // monitor, so the worst case degrades to the no-clamp fallback below.
        let monitorRect: { x: number; y: number; w: number; h: number } | null =
          currentMon != null
            ? {
                x: currentMon.position.x / scale,
                y: currentMon.position.y / scale,
                w: currentMon.size.width / scale,
                h: currentMon.size.height / scale,
              }
            : null;
        if (monitorRect == null) {
          const all = await availableMonitors();
          const hit = pickMonitorForPoint(
            all.map((m) => ({
              x: m.position.x,
              y: m.position.y,
              w: m.size.width,
              h: m.size.height,
            })),
            { x: pos.x, y: pos.y },
          );
          if (hit != null) {
            monitorRect = {
              x: hit.x / scale,
              y: hit.y / scale,
              w: hit.w / scale,
              h: hit.h / scale,
            };
          }
        }
        let anchor: MorphAnchor;
        let growsUp: boolean;
        let frameX: number;
        let frameY: number;
        if (monitorRect != null) {
          const target = computeExpandTarget(
            { x: iconX, y: iconY, size: MINIMIZED_WINDOW_SIZE },
            monitorRect,
            { w: fullWidth, h: fullHeight },
          );
          anchor = target.anchor;
          growsUp = target.growsUpward;
          frameX = target.x;
          frameY = target.y;
        } else {
          // No monitor geometry at all: keep the icon's top-left, no clamp.
          anchor = 'tl';
          growsUp = false;
          frameX = iconX;
          frameY = iconY;
        }
        morphAnchorRef.current = anchor;
        growsUpwardRef.current = growsUp;
        maxHeightRef.current = 0;
        windowPosRef.current = { x: frameX, bottomY: frameY + fullHeight };
        // eslint-disable-next-line @eslint-react/dom-no-flush-sync -- intentional: commit the anchor (and the invisible state) before the imperative native window move below.
        flushSync(() => {
          setMorphAnchor(anchor);
          setGrowsUpward(growsUp);
        });
        await yieldForPaint();
        void invoke('set_window_frame', {
          x: frameX,
          y: frameY,
          width: fullWidth,
          height: fullHeight,
        });
        setExpandReady(true);
        /* v8 ignore start -- defensive: geometry query only rejects on a
           real Tauri runtime failure, unreachable in the jsdom mock */
      } catch {
        morphAnchorRef.current = 'tl';
        growsUpwardRef.current = false;
        maxHeightRef.current = 0;
        // eslint-disable-next-line @eslint-react/dom-no-flush-sync -- intentional: commit invisible state before the native window resize (see above).
        flushSync(() => {
          setMorphAnchor('tl');
          setGrowsUpward(false);
        });
        await yieldForPaint();
        void invoke('animate_overlay_frame', {
          width: fullWidth,
          height: fullHeight,
          durationMs: 0,
        });
        setExpandReady(true);
      }
      /* v8 ignore stop */
    })();
  }, []);

  /**
   * Minimizes the overlay to the floating icon without cancelling generation.
   * The OS window stays at full chat size for the entire in-page tween; the
   * transparent NSPanel under a chat that has reached opacity 0 shows nothing,
   * so the user sees a clean chat → mascot handoff. The chat-card wrapper
   * shrinks toward its top-left corner (scale 1 → 0.34) + fades 1 → 0, while
   * the mascot springs in from scale 0.3 + opacity 0 to scale 1 + opacity 1
   * with a small overshoot. When the chat's animation settles,
   * `settleMorphPhase`
   * snaps the OS window to 48x48 (`durationMs:0`, invisible because the
   * painted content is already the mascot) and switches `morphPhase` to
   * `minimized`, which unmounts the chat subtree and lets clicks pass through
   * to the desktop around the 48px square.
   */
  const handleMinimize = useCallback(() => {
    // Only collapse from the settled chat/ask state. The minimize button stays
    // mounted throughout an expand (the chat subtree renders for every phase
    // except the settled `minimized` one), so a click mid-expand would jump
    // `morphPhase` from 'expanding' to 'collapsing' while that expand's pending
    // async window-frame write is still in flight, desyncing window geometry.
    // Mirrors handleRestore's `!== 'minimized'` guard.
    /* v8 ignore next -- unreachable in the jsdom harness: the framer-motion
       mock fires onAnimationComplete on mount, so the morph never lingers in a
       non-idle, non-minimized phase for a click to land on. Real in the
       browser, where the ~360ms expand tween runs with the button mounted. */
    if (morphPhaseRef.current !== 'idle') return;
    growsUpwardRef.current = false;
    setGrowsUpward(false);
    // Keep the panel key for the duration of the morph. It is a
    // nonactivating NSPanel, so WKWebView throttles requestAnimationFrame
    // (and with it Framer Motion's tween/spring clock) whenever the panel is
    // not the key window — which makes the collapse jump straight to the end
    // state and read as a hard cut rather than a morph. Focusing does not
    // activate the app (the panel stays nonactivating / Accessory policy);
    // it only keeps the animation clock running at 60fps.
    void getCurrentWindow().setFocus();
    setMorphPhase('collapsing');
    // Reset so the next expand re-enters the chat-held-collapsed prep state.
    setExpandReady(false);
    void invoke('set_overlay_minimized', { minimized: true });
  }, []);

  /**
   * Settles the in-flight morph to its terminal phase. Driven by BOTH the
   * transform wrapper's `onAnimationComplete` (the precise, normal path) and
   * a watchdog timer (the fallback when that callback never fires). Reads the
   * live `morphPhaseRef` and is idempotent: if the phase already settled, the
   * branches below are skipped, so a duplicate call (callback + watchdog both
   * firing, or a redirected animation firing twice) is harmless.
   *
   * - collapsing → the visible content is now just the 48px mascot at the
   *   window's top-left, so snap the OS window to that 48x48 square (instant,
   *   invisible) and settle to `minimized`, which unmounts the chat subtree
   *   and lets clicks pass through to the desktop.
   * - expanding → hand control back to the normal chat/ask state so the
   *   ResizeObserver resumes content-driven sizing.
   *
   * The `invoke` side effect lives outside the `setMorphPhase` updater (a
   * functional updater must be pure / Strict-Mode-double-invoke safe).
   */
  const settleMorphPhase = useCallback(() => {
    // Cancel any pending watchdog: whichever path (callback or timer) settles
    // first wins, and the other becomes a no-op.
    clearTimeout(morphSettleTimerRef.current);
    morphSettleTimerRef.current = 0;
    if (morphPhaseRef.current === 'collapsing') {
      // Chat is now at opacity 0 (visually gone). Settle to minimized
      // immediately (unmounts the chat subtree), then snap the OS window to
      // the 48x48 square at the active anchor's corner of the chat's CURRENT
      // frame, so the icon folds back to the exact screen spot the chat
      // unfolded from. The mascot is rendered at that same corner, so it stays
      // put through the resize. Reading the live frame here (rather than a
      // value captured at minimize start) handles a chat that was dragged
      // while expanded. The brief full-size-then-snap window is transparent
      // and shows only the mascot, so it is invisible.
      setMorphPhase('minimized');
      // Re-assert the Rust minimized flag at the collapse SETTLE.
      // `handleMinimize` set it at collapse START; if an activation arrived
      // mid-collapse, the activator consumed (cleared) the flag and emitted a
      // restore that `handleRestore` ignored (phase was still 'collapsing'),
      // leaving Rust=not-minimized while we now settle to minimized. Without
      // this re-assert the two sides disagree and the NEXT activation takes the
      // show/hide path, wiping the parked conversation. Re-asserting keeps them
      // in sync so the next activation correctly restores.
      void invoke('set_overlay_minimized', { minimized: true });
      const anchor = morphAnchorRef.current;
      void (async () => {
        try {
          const win = getCurrentWindow();
          const [pos, size, scale] = await Promise.all([
            win.outerPosition(),
            win.outerSize(),
            win.scaleFactor(),
          ]);
          const target = computeCollapseTarget(
            {
              x: pos.x / scale,
              y: pos.y / scale,
              w: size.width / scale,
              h: size.height / scale,
            },
            anchor,
            MINIMIZED_WINDOW_SIZE,
          );
          void invoke('set_window_frame', {
            x: target.x,
            y: target.y,
            width: MINIMIZED_WINDOW_SIZE,
            height: MINIMIZED_WINDOW_SIZE,
          });
          /* v8 ignore start -- defensive: geometry query only rejects on a
             real Tauri runtime failure, unreachable in the jsdom mock */
        } catch {
          void invoke('animate_overlay_frame', {
            width: MINIMIZED_WINDOW_SIZE,
            height: MINIMIZED_WINDOW_SIZE,
            durationMs: 0,
          });
        }
        /* v8 ignore stop */
      })();
      return;
    }
    if (morphPhaseRef.current === 'expanding') {
      setMorphPhase('idle');
      // The ResizeObserver does not re-fire on its own after the morph: the
      // wrapper's layout box is unchanged across collapse/expand (only its
      // CSS transform changed), and the observer callback is a no-op while
      // morphing. So the content-driven window height set on restore is never
      // corrected and the chat can stay clipped (off by the footer/padding,
      // and inconsistent run-to-run depending on incidental reflows). Force
      // one fresh observation on the next frame — after the idle render
      // commits so the morph guard has cleared — so the window snaps to the
      // true content height.
      /* v8 ignore start -- ResizeObserver re-observation is a browser-only
         correction; the jsdom mock never fires the observer callback, so
         there is no observable sizing behavior to assert here. */
      const node = layoutWrapperNodeRef.current;
      const observer = observerRef.current;
      if (node && observer) {
        requestAnimationFrame(() => {
          observer.unobserve(node);
          observer.observe(node);
        });
      }
      /* v8 ignore stop */
    }
  }, []);

  /**
   * Arms the watchdog that force-settles the morph if `onAnimationComplete`
   * does not fire within the animation duration plus a grace period. Any
   * previously armed timer is cleared first (clearTimeout(0) is a safe no-op
   * when none is pending), so only one watchdog is ever outstanding.
   */
  const scheduleMorphWatchdog = useCallback(
    (durationS: number) => {
      clearTimeout(morphSettleTimerRef.current);
      morphSettleTimerRef.current = window.setTimeout(
        settleMorphPhase,
        durationS * 1000 + MORPH_SETTLE_GRACE_MS,
      );
    },
    [settleMorphPhase],
  );

  // Arm the watchdog whenever a morph begins. A layout effect (not a passive
  // effect) runs during commit, BEFORE the transform wrapper's passive
  // `onAnimationComplete` effect, so in the normal case the precise callback
  // clears this timer immediately and it never fires. It only fires — and
  // force-settles the phase — when that callback is missed (identical
  // start/end target, or a stalled rAF clock on the nonactivating panel).
  useLayoutEffect(() => {
    if (morphPhase === 'collapsing') {
      scheduleMorphWatchdog(COLLAPSE_MORPH_DURATION_S);
    } else if (morphPhase === 'expanding' && expandReady) {
      // Arm the expand watchdog only once `expandReady` flips true — that is
      // when the grow tween actually begins. The expand defers the tween
      // behind a geometry-query IIFE (outerPosition + scaleFactor +
      // currentMonitor + a paint yield); arming at phase entry instead would
      // start the clock before the tween, so a slow/cold IPC round-trip could
      // let the watchdog force-settle to `idle` mid-tween and snap the chat in.
      scheduleMorphWatchdog(MORPH_DURATION_S);
    }
  }, [morphPhase, expandReady, scheduleMorphWatchdog]);

  // Clear any pending watchdog on unmount so it cannot fire into a torn-down
  // tree.
  useEffect(() => () => clearTimeout(morphSettleTimerRef.current), []);

  /** Ref attached to the chat-mode history dropdown for click-outside detection. */
  const historyDropdownRef = useRef<HTMLDivElement>(null);

  /** Ref attached to the chat-mode model picker dropdown for click-outside detection. */
  const modelPickerDropdownRef = useRef<HTMLDivElement>(null);
  /** Ref attached to the ask-bar mode model picker drawer for click-outside detection. */
  const modelPickerAskBarRef = useRef<HTMLDivElement>(null);

  /**
   * Close the model picker when the user clicks outside it, in either mode.
   * Clicks on any pill trigger (data-model-picker-toggle) are excluded so the
   * trigger's own onClick can manage the toggle without a double-close race.
   */
  useEffect(() => {
    if (!isModelPickerOpen) return;

    const handleMouseDown = (e: MouseEvent) => {
      const target = e.target as Element;
      if (
        modelPickerDropdownRef.current?.contains(target) ||
        modelPickerAskBarRef.current?.contains(target) ||
        target.closest?.('[data-model-picker-toggle]')
      ) {
        return;
      }
      setIsModelPickerOpen(false);
    };

    document.addEventListener('mousedown', handleMouseDown);
    return () => document.removeEventListener('mousedown', handleMouseDown);
  }, [isModelPickerOpen]);

  /** Toggles the history panel open/closed. Closes model picker (mutually exclusive). */
  const handleHistoryToggle = useCallback(() => {
    setIsHistoryOpen((prev) => !prev);
    setIsModelPickerOpen(false);
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
  const prevHeightRef = useRef<number>(COLLAPSED_WINDOW_HEIGHT);
  if (prevHistoryOpenRef.current && !isHistoryOpen) {
    setPendingNewConversation(false);
  }
  prevHistoryOpenRef.current = isHistoryOpen;

  // Detect when a streamed completion finishes while the overlay is minimized.
  // Uses the render-body ref pattern (not useEffect) to satisfy the
  // @eslint-react/set-state-in-effect lint rule, mirroring prevHistoryOpenRef above.
  const prevGeneratingRef = useRef(isGenerating);
  // Gate on `isSettledMinimized` (the parked-icon state), not `isMinimized`
  // (true throughout the collapse/expand morph too). A stream finishing during
  // an in-flight RESTORE would otherwise re-raise the "unseen" indicator the
  // restore just cleared, so it would wrongly show again on the next minimize.
  if (prevGeneratingRef.current && !isGenerating && isSettledMinimized) {
    setUnseenCompletion(true);
  }
  prevGeneratingRef.current = isGenerating;

  /**
   * When a submit flips the UI from ask-bar mode into chat mode while the
   * window is pinned near the bottom edge, animate the container from its
   * current height to the fixed full chat height. This is intentionally scoped
   * to the upward-growth path so the downward path remains unchanged.
   */
  useLayoutEffect(() => {
    /* v8 ignore start -- ResizeObserver + DOM mutations require a real browser */
    const container = morphingContainerNodeRef.current;
    const wasChatMode = previousIsChatModeRef.current;
    previousIsChatModeRef.current = isChatMode;

    if (!container) return;
    // Park during the minimize/restore morph: a height write here would
    // reflow the chat tree and fight the GPU transform (blank-sliver bug).
    if (isMinimized || isMorphingRef.current) return;
    if (!growsUpward || isHistoryOpen || !isChatMode || wasChatMode) {
      return;
    }

    const startHeight =
      container.offsetHeight > 0
        ? container.offsetHeight
        : prevHeightRef.current;
    container.style.transition = 'none';
    container.style.minHeight = '';
    container.style.height = `${startHeight}px`;
    void container.offsetHeight;

    const frameId = requestAnimationFrame(() => {
      // 0.4s and slightly softer cubic bezier specifically for upward morph
      container.style.transition = 'height 0.4s cubic-bezier(0.2, 0.8, 0.2, 1)';
      container.style.height = `${maxChatHeightRef.current}px`;
    });

    return () => cancelAnimationFrame(frameId);
    /* v8 ignore stop */
    // `isMinimized` is in deps so the early-return guard is re-evaluated when
    // the minimize/restore morph toggles it. When idle it is always false, so
    // the not-minimized chat/ask sizing path is unchanged.
  }, [growsUpward, isChatMode, isHistoryOpen, isMinimized]);

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
  useLayoutEffect(() => {
    /* v8 ignore start -- ResizeObserver + DOM mutations require a real browser */
    const container = morphingContainerNodeRef.current;
    if (!container) return;
    // Park during the minimize/restore morph: any min-height/height write
    // here reflows the chat tree and fights the GPU transform.
    if (isMinimized || isMorphingRef.current) return;

    // Track the height when we are NOT in chat mode natively.
    if (!isChatMode) {
      const h = container.offsetHeight;
      // offsetHeight might read 0 if hidden, so default to collapsed
      prevHeightRef.current = h > 0 ? h : COLLAPSED_WINDOW_HEIGHT;
      container.style.transition =
        'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
      container.style.height = '';
      container.style.minHeight = '';
      return;
    }

    if (!isHistoryOpen) {
      container.style.transition =
        'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
      container.style.minHeight = '';
      return;
    }

    const dropdown = historyDropdownRef.current;
    if (!dropdown) return;

    container.style.transition =
      'min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)';
    container.style.height = ''; // Let history panel dictate it via minHeight

    const sync = () => {
      container.style.minHeight = `${dropdown.offsetTop + dropdown.offsetHeight + 8}px`;
    };

    sync();
    const ro = new ResizeObserver(sync);
    ro.observe(dropdown);
    return () => ro.disconnect();
    /* v8 ignore stop */
    // `isMinimized` in deps re-evaluates the morph guard; no behavior change
    // for the not-minimized chat/ask path (always false there).
  }, [isChatMode, isHistoryOpen, isMinimized]);

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
        // `save` accepts `string | null` and short-circuits internally when
        // there is no active model, so the no-model guard lives in one
        // place rather than duplicated at every call site.
        await save(messages, activeModel);
      }
    } catch {
      // State stays unchanged on failure; feedback is implicit in the icon.
    }
  }, [isSaved, unsave, save, messages, activeModel]);

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
        setSearchActive(false);
      } catch {
        // Load failed - current session is preserved intact.
      } finally {
        setIsHistoryOpen(false);
      }
    },
    [loadConversation, loadMessages],
  );

  /**
   * Saves the current unsaved session then loads the requested conversation.
   *
   * If save fails the operation is aborted - we do not load the target
   * conversation because the current session has not been persisted yet.
   * If save succeeds but load fails the panel is still dismissed; the
   * current session has been saved so no data is lost.
   */
  const handleSaveAndLoad = useCallback(
    async (id: string) => {
      try {
        await save(messages, activeModel);
      } catch {
        // Save failed - abort to avoid leaving the current session unprotected.
        return;
      }
      try {
        const loaded = await loadConversation(id);
        loadMessages(loaded);
        setSearchActive(false);
      } catch {
        // Load failed - save already committed; dismiss panel, keep current view.
      } finally {
        setIsHistoryOpen(false);
      }
    },
    [save, messages, loadConversation, loadMessages, activeModel],
  );

  /**
   * Deletes a conversation from the history panel.
   *
   * When the deleted conversation is the currently active one, only the
   * persistence state (`resetHistory`) is cleared - messages remain visible
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
   */
  const resetForNewConversation = useCallback(() => {
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
    setSearchActive(false);
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
      await save(messages, activeModel);
    } catch {
      return;
    }
    resetForNewConversation();
  }, [save, messages, resetForNewConversation, activeModel]);

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
    // blob URL thumbnails immediately - keeps the UI responsive while
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
   * Root-level drag handlers. Attached to the `h-screen w-screen` root div so
   * file drops anywhere in the window are intercepted, including the
   * ConversationView area, which has no drop handlers of its own. Without this,
   * the WebView navigates to display the dropped image full-screen when the user
   * drops a second image after the first conversation turn.
   *
   * `dragover` must always call `e.preventDefault()` to signal the browser that
   * this element accepts drops; without it the `drop` event never fires.
   */
  const handleRootDragOver = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      if (isGenerating || isSubmitPending) return;
      setIsDragOver(
        attachedImages.length >= config.window.maxImages ? 'max' : 'normal',
      );
    },
    [
      isGenerating,
      isSubmitPending,
      attachedImages.length,
      config.window.maxImages,
    ],
  );

  const handleRootDragLeave = useCallback((e: React.DragEvent) => {
    // Only clear when the cursor truly exits the window. `dragleave` fires
    // when moving between child elements too; checking `relatedTarget` lets us
    // ignore those internal transitions.
    /* v8 ignore start -- dragleave relatedTarget cannot be set in jsdom; the false branch (cursor on child element) requires a real browser drag sequence */
    if (!(e.currentTarget as Element).contains(e.relatedTarget as Node)) {
      setIsDragOver(null);
    }
    /* v8 ignore stop */
  }, []);

  const handleRootDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragOver(null);
      if (isGenerating || isSubmitPending) return;
      const files = e.dataTransfer?.files;
      if (!files) return;
      const remaining = config.window.maxImages - attachedImages.length;
      if (remaining <= 0) return;
      const accepted: File[] = [];
      for (let i = 0; i < files.length && accepted.length < remaining; i++) {
        if (
          files[i].type.startsWith('image/') &&
          files[i].size <= MAX_IMAGE_SIZE_BYTES
        ) {
          accepted.push(files[i]);
        }
      }
      if (accepted.length > 0) handleImagesAttached(accepted);
    },
    [
      isGenerating,
      isSubmitPending,
      attachedImages.length,
      handleImagesAttached,
      config.window.maxImages,
    ],
  );

  /**
   * Invokes the Rust `capture_screenshot` command, which hides the window,
   * lets the user drag-select a screen region, then returns the captured image
   * as a base64 PNG string (or null if the user cancelled).
   * On success, converts the base64 to a File and feeds it into the existing
   * handleImagesAttached pipeline - identical to a paste or drag-drop.
   */
  const handleScreenshot = useCallback(async () => {
    /* v8 ignore start -- defensive guard: button is always disabled at max images, so this branch is unreachable through normal UI interaction */
    if (attachedImages.length >= config.window.maxImages) return;
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
  }, [attachedImages, handleImagesAttached, config.window.maxImages]);

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
    (submitQuery: string, context: string | undefined, think?: boolean) => {
      const readyPaths = attachedImages
        .filter((img) => img.filePath !== null)
        .map((img) => img.filePath as string);
      const images = readyPaths.length > 0 ? readyPaths : undefined;
      ask(submitQuery, context, images, think);
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
  const handleScreenSubmit = useCallback(
    async (fullQuery: string, think?: boolean, promptOverride?: string) => {
      const context = sanitizeContext(selectedContext, quote.maxContextLength);

      // Store the original input so handleCancel can restore it if the user
      // aborts the capture before it resolves.
      screenCaptureInputSnapshotRef.current = {
        query: fullQuery,
        context,
      };

      // Immediately show the user's message in chat with a loading placeholder
      // for the screenshot. This prevents double-submit spam and gives instant
      // feedback that the capture is in progress.
      screenCapturePendingRef.current = true;
      setIsSubmitPending(true);
      setPendingUserMessage(
        buildPendingBubble({
          query: fullQuery,
          context,
          attachedImages,
          hasScreen: true,
        }),
      );
      setQuery('');
      setSelectedContext(null);
      /* v8 ignore start -- inputRef always set when overlay is visible */
      if (inputRef.current) inputRef.current.style.height = 'auto';
      /* v8 ignore stop */

      let screenshotPath: string;
      try {
        screenshotPath = await invoke<string>('capture_full_screen_command', {
          conversationId: getTraceConversationId(),
        });
      } catch (e) {
        screenCapturePendingRef.current = false;
        screenCaptureInputSnapshotRef.current = null;
        // Capture failed: restore input state so the user can retry or edit.
        setIsSubmitPending(false);
        setPendingUserMessage(null);
        setQuery(fullQuery);
        setSelectedContext(context ?? null);
        // Surface the Rust error directly: the backend already provides
        // descriptive messages (permission prompts, null-image diagnostics, etc.).
        // Tauri v2 rejects with the Err(String) value as a plain string.
        setCaptureError(
          typeof e === 'string'
            ? e
            : e instanceof Error
              ? e.message
              : String(e),
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

      const readyPaths = resolveReadyPaths(attachedImages, screenshotPath);

      ask(fullQuery, context, readyPaths, think, promptOverride);
      for (const img of attachedImages) {
        URL.revokeObjectURL(img.blobUrl);
      }
      setAttachedImages([]);
    },
    [
      selectedContext,
      attachedImages,
      ask,
      getTraceConversationId,
      setSelectedContext,
      setCaptureError,
      quote.maxContextLength,
    ],
  );

  /**
   * Async handler for the `/extract` command path. Runs Vision OCR on all
   * attached images (and a fresh `/screen` capture if requested), then inserts
   * the result directly as an assistant message without calling Ollama.
   *
   * On Vision failure, falls back to Ollama if the active model supports vision;
   * otherwise surfaces a descriptive error via `captureError`.
   */
  const handleExtractSubmit = useCallback(
    async (fullQuery: string, hasScreen: boolean) => {
      const context = sanitizeContext(selectedContext, quote.maxContextLength);

      // Show the pending user bubble and lock out further submits.
      if (hasScreen) {
        screenCapturePendingRef.current = true;
        screenCaptureInputSnapshotRef.current = { query: fullQuery, context };
      }
      setIsSubmitPending(true);
      setPendingUserMessage(
        buildPendingBubble({
          query: fullQuery,
          context,
          attachedImages,
          hasScreen,
        }),
      );
      setQuery('');
      setSelectedContext(null);
      /* v8 ignore start -- inputRef always set when overlay is visible */
      if (inputRef.current) inputRef.current.style.height = 'auto';
      /* v8 ignore stop */

      // Capture screen if /screen is present.
      let screenshotPath: string | undefined;
      if (hasScreen) {
        try {
          screenshotPath = await invoke<string>('capture_full_screen_command', {
            conversationId: getTraceConversationId(),
          });
        } catch (e) {
          screenCapturePendingRef.current = false;
          screenCaptureInputSnapshotRef.current = null;
          setIsSubmitPending(false);
          setPendingUserMessage(null);
          setQuery(fullQuery);
          setSelectedContext(context ?? null);
          setCaptureError(
            typeof e === 'string'
              ? e
              : e instanceof Error
                ? e.message
                : String(e),
          );
          return;
        }

        const wasCancelled = !screenCapturePendingRef.current;
        screenCapturePendingRef.current = false;
        screenCaptureInputSnapshotRef.current = null;
        if (wasCancelled) return;
      }

      // Collect resolved image paths.
      const readyPaths = resolveReadyPaths(attachedImages, screenshotPath);

      // Clean up attached images.
      for (const img of attachedImages) URL.revokeObjectURL(img.blobUrl);
      setAttachedImages([]);

      // Resolve display paths for the real user bubble. handleSubmit only
      // dispatches /extract when an image is attached or /screen is used, and
      // the pending-image gate resolves paths before this point, so readyPaths
      // is always non-empty. The undefined fallback is defensive.
      /* v8 ignore start -- readyPaths non-empty invariant enforced by dispatch gate */
      const displayPaths = readyPaths.length > 0 ? readyPaths : undefined;
      /* v8 ignore stop */

      // Run Vision OCR.
      setCaptureError(null);
      let ocrText: string;
      try {
        ocrText = await invoke<string>('extract_text_command', {
          imagePaths: readyPaths,
        });
      } catch (e) {
        // Vision failed: try Ollama if the active model supports vision.
        const hasVision = activeModelCapabilities?.vision ?? false;
        setIsSubmitPending(false);
        setPendingUserMessage(null);
        if (hasVision && readyPaths.length > 0) {
          ask(
            fullQuery,
            context,
            readyPaths,
            undefined,
            'Extract all text visible in this image verbatim. Output only the extracted text with no commentary, preamble, or formatting.',
          );
        } else {
          setQuery(fullQuery);
          setSelectedContext(context ?? null);
          setCaptureError(
            `OCR failed${
              typeof e === 'string'
                ? `: ${e}`
                : e instanceof Error
                  ? `: ${e.message}`
                  : ''
            }. Switch to a vision-capable model to try via Ollama.`,
          );
        }
        return;
      }

      // Success: insert the turn directly without an Ollama call.
      setIsSubmitPending(false);
      setPendingUserMessage(null);
      addOcrTurn(
        fullQuery,
        context,
        displayPaths,
        `\`\`\`\n${ocrText}\n\`\`\``,
      );
    },
    [
      selectedContext,
      attachedImages,
      ask,
      addOcrTurn,
      getTraceConversationId,
      setSelectedContext,
      setCaptureError,
      quote.maxContextLength,
      activeModelCapabilities,
    ],
  );

  /**
   * Async handler for utility commands (`/tldr`, `/translate`, etc.) when the
   * user has attached images or added `/screen`. Runs Vision OCR to extract
   * text, builds the prompt template with the OCR result as `$INPUT`, then
   * calls `ask()` without sending any image bytes to the model.
   */
  const handleUtilityOcrSubmit = useCallback(
    async (
      fullQuery: string,
      trigger: string,
      strippedMessage: string,
      hasScreen: boolean,
      hasThink: boolean,
    ) => {
      const context = sanitizeContext(selectedContext, quote.maxContextLength);

      // Show the pending user bubble and lock out further submits.
      if (hasScreen) {
        screenCapturePendingRef.current = true;
        screenCaptureInputSnapshotRef.current = { query: fullQuery, context };
      }
      setIsSubmitPending(true);
      setPendingUserMessage(
        buildPendingBubble({
          query: fullQuery,
          context,
          attachedImages,
          hasScreen,
        }),
      );
      setQuery('');
      setSelectedContext(null);
      /* v8 ignore start -- inputRef always set when overlay is visible */
      if (inputRef.current) inputRef.current.style.height = 'auto';
      /* v8 ignore stop */

      // Capture screen if /screen is present.
      let screenshotPath: string | undefined;
      if (hasScreen) {
        try {
          screenshotPath = await invoke<string>('capture_full_screen_command', {
            conversationId: getTraceConversationId(),
          });
        } catch (e) {
          screenCapturePendingRef.current = false;
          screenCaptureInputSnapshotRef.current = null;
          setIsSubmitPending(false);
          setPendingUserMessage(null);
          setQuery(fullQuery);
          setSelectedContext(context ?? null);
          /* v8 ignore start -- Tauri always rejects with string or Error */
          setCaptureError(
            typeof e === 'string'
              ? e
              : e instanceof Error
                ? e.message
                : String(e),
          );
          /* v8 ignore stop */
          return;
        }

        const wasCancelled = !screenCapturePendingRef.current;
        screenCapturePendingRef.current = false;
        screenCaptureInputSnapshotRef.current = null;
        if (wasCancelled) return;
      }

      // Collect resolved image paths (dispatch guard already filtered out
      // any still-pending images via the pre-flight gate).
      const readyPaths = resolveReadyPaths(attachedImages, screenshotPath);

      // Clean up attached images.
      for (const img of attachedImages) URL.revokeObjectURL(img.blobUrl);
      setAttachedImages([]);

      // Run Vision OCR.
      let ocrText: string;
      try {
        ocrText = await invoke<string>('extract_text_command', {
          imagePaths: readyPaths,
        });
      } catch (e) {
        setIsSubmitPending(false);
        setPendingUserMessage(null);
        setQuery(fullQuery);
        setSelectedContext(context ?? null);
        /* v8 ignore start -- Tauri always rejects with string or Error */
        setCaptureError(
          `OCR failed${
            typeof e === 'string'
              ? `: ${e}`
              : e instanceof Error
                ? `: ${e.message}`
                : ''
          }`,
        );
        /* v8 ignore stop */
        return;
      }

      if (ocrText === '[No text detected]') {
        setIsSubmitPending(false);
        setPendingUserMessage(null);
        setQuery(fullQuery);
        setSelectedContext(context ?? null);
        setCaptureError('No readable text found in the image.');
        return;
      }

      // Build the prompt with OCR text as $INPUT.
      const composedPrompt = buildPrompt(trigger, strippedMessage, ocrText);
      /* v8 ignore next 6 */
      if (!composedPrompt) {
        setIsSubmitPending(false);
        setPendingUserMessage(null);
        setQuery(fullQuery);
        setSelectedContext(context ?? null);
        return;
      }

      // Fire the ask — no image bytes reach the model, but display paths
      // flow to the user bubble so the thumbnail is visible in the conversation.
      setIsSubmitPending(false);
      setPendingUserMessage(null);
      ask(
        fullQuery,
        context,
        undefined,
        hasThink || undefined,
        composedPrompt,
        /* v8 ignore next -- dispatch guard ensures at least one image source; empty path is defensive */
        readyPaths.length > 0 ? readyPaths : undefined,
      );
    },
    [
      selectedContext,
      attachedImages,
      ask,
      getTraceConversationId,
      setSelectedContext,
      setCaptureError,
      quote.maxContextLength,
    ],
  );

  /**
   * Live strip message for the current environment + compose state. Drives
   * the inline `CapabilityMismatchStrip` so the user sees the right cue as
   * soon as content lands in compose, not only at submit time. The strip
   * is purely informational: recovery happens through the model picker
   * chip (or starting Ollama, when that is the actual problem).
   *
   * Resolution order matters: environment-state messaging wins over
   * capability conflicts because telling the user to "switch models"
   * makes no sense when Ollama is down or has no models installed. Once
   * an active model exists and Ollama is reachable, fall through to the
   * per-message capability check.
   */
  /**
   * History-state derived from the current `messages` array. Drives the
   * Phase B history-based capability strip: a heads-up when the active
   * model lacks a capability earlier turns relied on (vision, thinking).
   * `messages.some(...)` is O(n) per render but bounded by typical chat
   * lengths and memoized against the messages reference.
   */
  const historyCapabilityState = useMemo(() => {
    let maxImages = 0;
    let hasThinking = false;
    for (const m of messages) {
      const count = m.imagePaths?.length ?? 0;
      if (count > maxImages) maxImages = count;
      if ((m.thinkingContent?.length ?? 0) > 0) hasThinking = true;
    }
    return {
      historyHasImages: maxImages > 0,
      historyHasThinking: hasThinking,
      historyMaxImagesPerMessage: maxImages,
    };
  }, [messages]);

  /**
   * Compose-state slice the conflict gate consumes. Recomputed only when
   * the underlying inputs change so downstream memos can short-circuit on
   * reference equality. Pulled out so the live conflict memo and the
   * submit-time shake gate can share one source of truth and never drift.
   */
  const composeCapabilityState = useMemo(() => {
    const trimmed = query.trim();
    const { found } = parseCommands(trimmed);
    const hasExtractCommand = found.has('/extract');
    const hasScreenCommand = found.has('/screen');
    const hasUtilityCommand = Array.from(found).some((t) => {
      const cmd = COMMANDS.find((c) => c.trigger === t);
      return !!cmd?.promptTemplate;
    });
    // /extract and utility+image/screen route through Vision OCR; suppress
    // image and screen counts so the capability gate does not block the submit.
    const ocrPath =
      hasExtractCommand ||
      (hasUtilityCommand && (attachedImages.length > 0 || hasScreenCommand));
    return {
      hasScreenCommand: ocrPath ? false : hasScreenCommand,
      hasThinkCommand: found.has('/think'),
      imageCount: ocrPath ? 0 : attachedImages.length,
    };
  }, [query, attachedImages]);

  const liveCapabilityConflictMessage = useMemo(() => {
    const envMessage = getEnvironmentMessage(
      ollamaReachable,
      availableModels.length,
      activeModel,
    );
    if (envMessage !== null) return envMessage;
    return getCapabilityConflict(
      activeModel,
      activeModelCapabilities,
      composeCapabilityState,
      historyCapabilityState,
    );
  }, [
    composeCapabilityState,
    historyCapabilityState,
    activeModel,
    activeModelCapabilities,
    ollamaReachable,
    availableModels.length,
  ]);

  /**
   * Submit-time shake gate. Shakes on compose-state conflicts (image
   * attached to text-only model, /think on non-thinking, multi-image
   * overflow) and on environment-state conflicts (Ollama unreachable,
   * no models installed, no active model). History-only conflicts
   * inform via the strip but never shake; the backend per-request
   * filter strips incompatible content so submit keeps working, and
   * shaking every turn until the user switches models would trap them
   * in the conversation.
   */
  const hasBlockingConflict = useMemo(() => {
    const envMessage = getEnvironmentMessage(
      ollamaReachable,
      availableModels.length,
      activeModel,
    );
    if (envMessage !== null) return true;
    return isComposeCapabilityConflict(
      activeModelCapabilities,
      composeCapabilityState,
    );
  }, [
    ollamaReachable,
    availableModels.length,
    activeModel,
    activeModelCapabilities,
    composeCapabilityState,
  ]);

  /**
   * Opens the macOS save dialog, serialises the current session as a
   * self-contained Markdown file (frontmatter + role-labelled blocks +
   * inline base64 images via the Tauri asset protocol), and asks the
   * Rust backend to write it to the user's chosen path.
   *
   * No-ops on an empty session (defensive — the slash-command path is
   * gated upstream and the chat-header button only renders in chat mode).
   * User cancellation of the save dialog returns silently. A failed
   * write surfaces the OS-level error message via the existing
   * `captureError` banner.
   */
  const runFileExport = useCallback(async () => {
    setIsExportOpen(false);
    /* v8 ignore start -- defensive: callers gate on messages.length > 0 */
    if (messages.length === 0) return;
    /* v8 ignore stop */
    // Hide Thuki via NSPanel alpha while the native save dialog is on
    // screen. The dialog's drop-shadow and vibrancy backdrop would
    // otherwise bleed onto Thuki's transparent shadow margin and render
    // as a dark "ghost" rectangle around the card.
    //
    // Both `set_overlay_alpha` calls are fired without `await` so the
    // main thread sees them dispatched in the same event-loop tick as
    // the save-dialog command. Awaiting alpha serially introduces a
    // visible "Thuki invisible, dialog not yet appearing" frame that
    // reads as a glitch; the fire-and-forget shape collapses that gap.
    // The setter is a thin Rust function that cannot fail in
    // practice — IPC bus rejection would be the only path — so an
    // unhandled rejection is acceptable.
    try {
      // Hide instantly — the dialog's own appear animation is the
      // motion the user reads, so a snap-out keeps the transition
      // crisp from Thuki → dialog.
      void invoke('set_overlay_alpha', { alpha: 0, durationMs: 0 });
      const path = await saveDialog({
        defaultPath: defaultExportFilename(new Date()),
        filters: [{ name: 'Markdown', extensions: ['md'] }],
      });
      if (path === null) return;
      const content = await serializeForFile(
        messages,
        { fallbackModel: activeModel },
        new Date(),
      );
      await invoke('save_chat_export', { path, content });
    } catch (err) {
      setCaptureError(
        `Failed to export: ${err instanceof Error ? err.message : String(err)}`,
      );
    } finally {
      // Fade back in over 150 ms so Thuki re-emerges in step with the
      // dialog's dismiss animation instead of snapping in late.
      void invoke('set_overlay_alpha', { alpha: 1, durationMs: 150 });
    }
  }, [messages, activeModel]);

  /**
   * Copies the current session to the system clipboard as body-only
   * Markdown. Strips the YAML frontmatter (would surface as visible
   * noise in chat apps) and substitutes image markers for screenshots
   * (a multi-megabyte base64 payload would otherwise jam most paste
   * targets). Errors surface via the `captureError` banner.
   */
  const runClipboardCopy = useCallback(async () => {
    setIsExportOpen(false);
    /* v8 ignore start -- defensive: callers gate on messages.length > 0 */
    if (messages.length === 0) return;
    /* v8 ignore stop */
    try {
      const content = serializeForClipboard(messages);
      await navigator.clipboard.writeText(content);
    } catch (err) {
      setCaptureError(
        `Failed to copy: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  }, [messages]);

  /**
   * Toggles the export popover from the chat-header button. Closes the
   * model-picker dropdown when opening so the two popovers (anchored to
   * the same `right-3 top-10` corner of the chat header) never overlap.
   */
  const handleExportToggle = useCallback(() => {
    setIsExportOpen((open) => {
      if (!open) {
        setIsModelPickerOpen(false);
      }
      return !open;
    });
  }, []);

  /**
   * Dismisses the export popover when the user clicks outside it. The
   * toggle button itself is excluded so the click that already toggled
   * the popover does not also close it on the same gesture.
   */
  useEffect(() => {
    if (!isExportOpen) return;
    const handler = (event: MouseEvent) => {
      const target = event.target;
      /* v8 ignore start -- happy-dom always yields a Node target */
      if (!(target instanceof Node)) return;
      /* v8 ignore stop */
      const popover = exportPopoverRef.current;
      /* v8 ignore start -- the ref is attached whenever the popover renders */
      if (popover === null) return;
      /* v8 ignore stop */
      if (popover.contains(target)) return;
      if (target instanceof Element && target.closest('[data-export-toggle]')) {
        return;
      }
      setIsExportOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [isExportOpen]);

  const handleSubmit = useCallback(() => {
    if (
      (query.trim().length === 0 && attachedImages.length === 0) ||
      isGenerating ||
      isSubmitPending
    )
      return;

    // Clear any stale capture error from a previous attempt.
    setCaptureError(null);

    // Parse all valid commands from anywhere in the message.
    const trimmedQuery = query.trim();
    const { found, strippedMessage } = parseCommands(trimmedQuery);
    const hasScreen = found.has('/screen');
    const hasThink = found.has('/think');
    const hasSearch = found.has('/search');
    const hasExtract = found.has('/extract');
    const utilityTrigger = Array.from(found).find((t) => {
      const cmd = COMMANDS.find((c) => c.trigger === t);
      return !!cmd?.promptTemplate;
    });

    // /extract requires content to extract from. Shake before any other gate
    // so the error message is the specific one.
    if (hasExtract && attachedImages.length === 0 && !hasScreen) {
      setCaptureError('Attach an image or add /screen to extract text from.');
      setShakeAskBar(true);
      return;
    }

    // /export is a session-level command. It must run in isolation from any
    // other command in the same message and is gated on "at least one
    // message exists". Same shake + banner UX as /extract for consistency.
    const hasExport = found.has('/export');
    if (hasExport) {
      if (messages.length === 0) {
        setCaptureError('No messages to export yet.');
        setShakeAskBar(true);
        return;
      }
      setQuery('');
      /* v8 ignore next */
      inputRef.current!.style.height = 'auto';
      void runFileExport();
      return;
    }

    // OCR paths (/extract, utility commands with images or /screen) bypass
    // the Ollama capability/environment gate: Vision OCR runs locally and
    // utility OCR sends extracted text — never image bytes — to the model.
    // The `composeCapabilityState` memo also suppresses image/screen counts
    // for these paths, but we short-circuit here to make the intent explicit
    // and to skip the env gate for /extract specifically.
    const isOcrPath =
      hasExtract ||
      (utilityTrigger !== undefined &&
        (hasScreen || attachedImages.length > 0));

    // Submit-time capability gate. Refuses messages whose attached content
    // the active model cannot handle (images on a text-only model) and
    // environment-state failures (Ollama unreachable, no model selected).
    // History-only mismatches do NOT shake: the backend filter strips
    // incompatible content from the per-request snapshot, so submit keeps
    // working and the strip already explains what is happening.
    if (!isOcrPath && hasBlockingConflict) {
      setShakeAskBar(true);
      return;
    }

    // `/search` entry point AND sticky follow-ups. Search ignores attached
    // images entirely (the pipeline never sends image bytes), so it does
    // NOT route through the pending-images gate below. An explicit /screen
    // command takes precedence over search continuation so users can always
    // attach a screenshot mid-conversation.
    if (hasSearch || (searchActive && !hasScreen && found.size === 0)) {
      const searchQuery = strippedMessage.trim();
      if (!searchQuery) return;
      const searchContext = sanitizeContext(
        selectedContext,
        quote.maxContextLength,
      );
      // Pass the full typed query (with `/search`) as bubble display content so
      // the user sees exactly what they typed; the backend receives only the
      // stripped query without the trigger prefix.
      const searchDisplay = hasSearch ? trimmedQuery : undefined;
      setQuery('');
      setSelectedContext(null);
      /* v8 ignore next */
      inputRef.current!.style.height = 'auto';
      setSearchActive(true);
      void askSearch(searchQuery, searchDisplay, searchContext).then(
        ({ final }) => {
          if (final) setSearchActive(false);
        },
      );
      return;
    }

    // Nothing to send if the message is only commands with no content or images.
    // Utility triggers are excluded: they fall through to their own block below
    // which shakes + shows an error when no input is found.
    // Exception: /think with pre-filled selected context is valid.
    if (
      !strippedMessage &&
      attachedImages.length === 0 &&
      !hasScreen &&
      !utilityTrigger &&
      !(hasThink && selectedContext?.trim())
    )
      return;

    const context = sanitizeContext(selectedContext, quote.maxContextLength);

    // Unified pre-flight pending-images gate. Every command that needs
    // resolved image paths waits here: /extract, /screen, utility-OCR, and
    // plain submit. If any attached image is still processing, store the
    // submit intent in `pendingSubmitRef` keyed by command kind; the effect
    // below dispatches to the matching stage-2 handler once every image
    // has a resolved `filePath`. Without this gate, handlers would filter
    // pending images out and silently produce empty-input errors.
    const hasPendingImages = attachedImages.some(
      (img) => img.filePath === null,
    );
    if (hasPendingImages) {
      setIsSubmitPending(true);
      setPendingUserMessage(
        buildPendingBubble({
          query: trimmedQuery,
          context,
          attachedImages,
          hasScreen,
        }),
      );
      setQuery('');
      /* v8 ignore start -- inputRef always set when overlay is visible */
      if (inputRef.current) inputRef.current.style.height = 'auto';
      /* v8 ignore stop */

      if (hasExtract) {
        pendingSubmitRef.current = {
          kind: 'extract',
          query: trimmedQuery,
          context,
          hasScreen,
        };
      } else if (utilityTrigger) {
        pendingSubmitRef.current = {
          kind: 'utility-ocr',
          query: trimmedQuery,
          context,
          think: hasThink,
          trigger: utilityTrigger,
          strippedMessage,
          hasScreen,
        };
      } else if (hasScreen) {
        pendingSubmitRef.current = {
          kind: 'screen',
          query: trimmedQuery,
          context,
          think: hasThink,
        };
      } else {
        pendingSubmitRef.current = {
          kind: 'plain',
          query: trimmedQuery,
          context,
          think: hasThink,
        };
      }
      return;
    }

    // Direct dispatch: all attached images (if any) are already resolved.

    if (hasExtract) {
      void handleExtractSubmit(trimmedQuery, hasScreen);
      return;
    }

    if (utilityTrigger && (hasScreen || attachedImages.length > 0)) {
      void handleUtilityOcrSubmit(
        trimmedQuery,
        utilityTrigger,
        strippedMessage,
        hasScreen,
        hasThink,
      );
      return;
    }

    if (hasScreen) {
      void handleScreenSubmit(trimmedQuery, hasThink, undefined);
      return;
    }

    if (utilityTrigger) {
      // Text-only utility command (no images, no /screen).
      const composedPrompt = buildPrompt(
        utilityTrigger,
        strippedMessage,
        context,
      );
      if (!composedPrompt) {
        setCaptureError(
          `Provide text or attach an image to use ${utilityTrigger}.`,
        );
        setShakeAskBar(true);
        return;
      }
      ask(
        trimmedQuery,
        context,
        undefined,
        hasThink || undefined,
        composedPrompt,
      );
      setSelectedContext(null);
      setQuery('');
      /* v8 ignore next */
      inputRef.current!.style.height = 'auto';
      return;
    }

    executeSubmit(trimmedQuery, context, hasThink || undefined);
  }, [
    query,
    isGenerating,
    isSubmitPending,
    executeSubmit,
    handleScreenSubmit,
    handleExtractSubmit,
    handleUtilityOcrSubmit,
    selectedContext,
    setSelectedContext,
    attachedImages,
    setCaptureError,
    ask,
    askSearch,
    searchActive,
    quote.maxContextLength,
    hasBlockingConflict,
    messages.length,
    runFileExport,
  ]);

  // When a pending submit exists and all images finish processing, dispatch
  // to the matching stage-2 handler. Reads `attachedImages` directly (not
  // through closure) to guarantee the effect always sees the freshest file
  // paths. The switch on `pendingSubmitRef.current.kind` is the single
  // place that maps a deferred command back to its handler — adding a new
  // image-aware command means adding a `kind` variant and one branch here.
  /* eslint-disable @eslint-react/set-state-in-effect -- intentional: effect
     reacts to image processing completion and must synchronously transition
     state (pending → submitted) in the same tick to avoid stale renders. */
  useEffect(() => {
    const ref = pendingSubmitRef.current;
    if (!ref) return;
    if (attachedImages.length === 0) {
      // All images failed - restore the user's query so their text isn't lost.
      pendingSubmitRef.current = null;
      setIsSubmitPending(false);
      setPendingUserMessage(null);
      setQuery(ref.query);
      setSelectedContext(ref.context ?? null);
      return;
    }
    // Wait until every image has finished backend processing.
    const allReady = attachedImages.every((img) => img.filePath !== null);
    if (!allReady) return;

    pendingSubmitRef.current = null;

    switch (ref.kind) {
      case 'extract':
        // Clear the loading-spinner bubble; handleExtractSubmit re-sets a
        // fresh pending bubble synchronously before its first await so
        // React batches the two state updates with no visible flash.
        setPendingUserMessage(null);
        void handleExtractSubmit(ref.query, ref.hasScreen);
        return;
      case 'utility-ocr':
        setPendingUserMessage(null);
        void handleUtilityOcrSubmit(
          ref.query,
          ref.trigger,
          ref.strippedMessage,
          ref.hasScreen,
          ref.think,
        );
        return;
      case 'screen':
        setPendingUserMessage(null);
        void handleScreenSubmit(ref.query, ref.think, undefined);
        return;
      case 'plain': {
        setIsSubmitPending(false);
        // Clear the preview message - ask() will add the real one with file paths.
        setPendingUserMessage(null);
        const images = attachedImages.map((img) => img.filePath as string);
        void ask(ref.query, ref.context, images, ref.think || undefined);
        setSelectedContext(null);
        for (const img of attachedImages) {
          URL.revokeObjectURL(img.blobUrl);
        }
        setAttachedImages([]);
        return;
      }
    }
  }, [
    attachedImages,
    ask,
    handleExtractSubmit,
    handleUtilityOcrSubmit,
    handleScreenSubmit,
    setSelectedContext,
  ]);
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
    void cancel();
    setSearchActive(false);
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [isSubmitPending, cancel, setSearchActive, setSelectedContext]);

  /**
   * Persists the user's model choice via the backend and closes the picker panel.
   * On rejection (e.g. the chosen model was uninstalled between render and click),
   * triggers a refresh so the picker list and the active chip resync with the
   * actual backend state instead of silently drifting.
   */
  const handleModelSelect = useCallback(
    (model: string) => {
      setIsModelPickerOpen(false);
      void setActiveModel(model).catch(() => {
        void refreshModels();
      });
    },
    [setActiveModel, refreshModels],
  );

  /** Closes the model picker panel. Wired to Escape key inside the panel. */
  const handleModelPickerClose = useCallback(() => {
    setIsModelPickerOpen(false);
  }, []);

  /**
   * Toggles the model picker panel. Closes history panel (mutually exclusive).
   *
   * On open we re-pull both the installed-model list and the per-model
   * capability map so newly-pulled models (e.g. user ran `ollama pull
   * deepseek-r1:1.5b` while Thuki was running) appear with their full
   * capability label without needing an app restart. Backend
   * `reconcile_capabilities` honors its cache for already-known slugs and
   * only fetches `/api/show` for genuinely new entries, so this is cheap.
   */
  const handleModelPickerToggle = useCallback(() => {
    setIsModelPickerOpen((prev) => {
      const opening = !prev;
      if (opening) {
        void refreshModels();
        void refreshModelCapabilities();
      }
      return opening;
    });
    setIsHistoryOpen(false);
  }, [refreshModels, refreshModelCapabilities]);

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
          if (payload.state === 'restore') {
            handleRestore();
            return;
          }
          if (payload.state === 'show') {
            replayEntranceAnimation(
              payload.selected_text ?? null,
              payload.window_x ?? null,
              payload.window_y ?? null,
              payload.screen_bottom_y ?? null,
            );
            return;
          }
          requestHideOverlay();
        },
      );
      unlistenOnboarding = await listen<{ stage: OnboardingStage }>(
        ONBOARDING_EVENT,
        ({ payload }) => {
          setOnboardingStage(payload.stage);
        },
      );
      // Both listeners registered - safe to let Rust decide what to show on launch.
      await invoke('notify_frontend_ready');
    };

    void attachListeners();
    return () => {
      unlistenVisibility?.();
      unlistenOnboarding?.();
    };
  }, [handleRestore, replayEntranceAnimation, requestHideOverlay]);

  /**
   * Combined close handler shared by the keyboard shortcut (Esc/Cmd+W)
   * and the traffic light close/minimize buttons. Notifies the Rust
   * backend and triggers the frontend exit animation sequence.
   */
  const handleCloseOverlay = useCallback(() => {
    void invoke('notify_overlay_hidden');
    requestHideOverlay();
  }, [requestHideOverlay]);

  /** Hide window on Escape or Cmd+W (macOS) / Ctrl+W. No-op while minimized. */
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (isMinimized) return;
      if (((e.metaKey || e.ctrlKey) && e.key === 'w') || e.key === 'Escape') {
        e.preventDefault();
        handleCloseOverlay();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [handleCloseOverlay, isMinimized]);

  /**
   * Programmatic focus when the overlay is visible and in the normal (idle)
   * chat/ask state. Keyed on `morphPhase` as well as `overlayState` so a
   * restore — which settles `minimized → idle` without `overlayState` ever
   * leaving 'visible' — refocuses the composer. Without the `morphPhase`
   * dependency the textarea stayed unfocused after restoring a parked chat.
   */
  useEffect(() => {
    if (overlayState === 'visible' && morphPhase === 'idle') {
      const raf = requestAnimationFrame(() => inputRef.current?.focus());
      return () => cancelAnimationFrame(raf);
    }
  }, [overlayState, morphPhase]);

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

    // After the user repositions the window, drop the upward-grow mode so
    // subsequent conversation growth tracks the new position downward.
    window.addEventListener(
      'mouseup',
      () => {
        growsUpwardRef.current = false;
        setGrowsUpward(false);
      },
      { once: true },
    );
  }, []);

  /**
   * In-page morph target for the chat-card transform wrapper. The morph is
   * GPU transforms only (scale/x/y/opacity), never width/height/layout,
   * so the heavy chat tree never reflows and there is no blank frame. The
   * OS window does NOT resize during the animation; it only snap-resizes at
   * the endpoints (durationMs:0) when the painted content already matches.
   *
   * `transformOrigin: top left` keeps the card's top-left corner pinned, so
   * scaling alone makes the card visibly shrink toward the corner where the
   * 48px window will snap. The exact scale/translate/curve are intentionally
   * approximate and meant to be fine-tuned on-device.
   *
   * `collapsing` and `minimized` share the collapsed target so the wrapper
   * does not snap back to identity mid-AnimatePresence-swap.
   */
  // The chat-card collapse target: shrink the card down toward its top-left
  // corner (transformOrigin: top-left = the corner where the 68px mascot
  // lands) while fading out. The scale travel is large (down to ~0.34) so
  // the card visibly funnels into the corner rather than just fading in
  // place; that travel is what makes the collapse read as a morph. The old
  // "tiny readable chat thumbnail" artifact is avoided by fading opacity to
  // 0 well before the shrink finishes (see morphTransition: opacity runs at
  // ~0.55x the scale duration), so by the time the card is small it is
  // already invisible. We do NOT scale all the way to
  // `MINIMIZED_WINDOW_SIZE / overlayWidth` (~0.06) because the visible part
  // of the shrink ends once opacity hits 0, so any smaller target only
  // affects the invisible tail.
  // 'collapsing' and 'minimized' share the same target so the wrapper does
  // not snap back to identity at the phase boundary; the chat subtree is
  // also unmounted during 'minimized' so the visible result at both phases
  // is just the portaled mascot.
  const COLLAPSED_SCALE = 0.34;
  // Tailwind inset classes that pin the floating mascot to the active anchor
  // corner of the (portaled, viewport-fixed) window. For the 68px settled
  // window all four corners coincide, so this also reads correctly minimized;
  // it only matters visually while the full-size window resizes under the
  // mascot during the morph, where the anchored corner stays on the icon.
  const mascotCornerClass = {
    tl: 'top-0 left-0',
    tr: 'top-0 right-0',
    bl: 'bottom-0 left-0',
    br: 'bottom-0 right-0',
  }[morphAnchor];
  // The chat card is collapsed (small + invisible) while collapsing, settled
  // minimized, AND during the first part of expand (before expandReady) — that
  // last case holds it invisible across the native window move so it cannot
  // flash at the wrong position. It grows out of the anchor corner only once
  // expandReady flips true, after the window has been repositioned.
  const chatCollapsed =
    morphPhase === 'collapsing' ||
    morphPhase === 'minimized' ||
    (morphPhase === 'expanding' && !expandReady);
  const morphTransform = chatCollapsed
    ? { scale: COLLAPSED_SCALE, opacity: 0 }
    : { scale: 1, opacity: 1 };
  // Per-property timing: opacity fades faster than the scale shrinks so the
  // card turns transparent before it gets small enough to read as a
  // thumbnail. `onAnimationComplete` fires when the longest property (scale)
  // settles, so the window snap to 48x48 still happens after the full
  // collapse. Collapse uses the longer, more cinematic duration; expand
  // keeps the snappier `MORPH_DURATION_S` so only the minimize direction is
  // slowed down.
  const morphDuration =
    morphPhase === 'collapsing' ? COLLAPSE_MORPH_DURATION_S : MORPH_DURATION_S;
  const morphTransition = {
    scale: { duration: morphDuration, ease: MORPH_EASE },
    opacity: { duration: morphDuration * 0.55, ease: MORPH_EASE },
  };
  // The mascot's own entrance/exit animation is CSS-driven (see the
  // `.thuki-mascot*` rules in App.css and the portal JSX below), not Framer
  // Motion, so it is painted by the compositor even when the nonactivating
  // panel loses key focus and rAF is throttled.

  if (onboardingStage !== null) {
    return (
      <OnboardingView
        stage={onboardingStage}
        onComplete={() => setOnboardingStage(null)}
      />
    );
  }

  return (
    // Minimal padding (pt-2 pb-6) provides just enough physical clearance for the
    // tightened drop shadow to render without clipping at the native window edge.
    <div
      onMouseDown={handleDragStart}
      onDragOver={handleRootDragOver}
      onDragLeave={handleRootDragLeave}
      onDrop={handleRootDrop}
      className={`flex flex-col items-center ${growsUpward ? 'justify-end' : 'justify-start'} h-screen w-screen ${isSettledMinimized ? '' : 'px-3 pt-2 pb-6'} bg-transparent overflow-visible`}
    >
      <AnimatePresence mode="wait">
        {shouldRenderOverlay ? (
          <motion.div
            key={`overlay-${sessionId}`}
            initial={{ opacity: 0, y: -20, scale: 0.96 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -16, scale: 0.98 }}
            transition={{ type: 'spring', stiffness: 260, damping: 24 }}
            className={
              isSettledMinimized
                ? 'overflow-visible'
                : 'w-full px-4 py-2 overflow-visible'
            }
          >
            {/* Relative wrapper - positioning context for absolute-positioned
                dropdowns (history, model picker) so they can float above the
                chat without being clipped. Also the positioning context for
                the morph mascot overlay (absolutely pinned to the top-left,
                where the 48px window snaps when settled-minimized). */}
            <div className="relative">
              {/* Layout wrapper: provides visual appearance (background, border,
                  border-radius, shadow) and is observed by ResizeObserver so the
                  native window tracks the combined height of the chat area and the
                  footer slot. The inner morphing container clips content during the
                  morph animation; the footer slot sits outside it so it is never
                  clipped by overflow-hidden when chat is at max height.

                  During the minimize/restore morph this wrapper is the GPU
                  transform target: it scales down + crossfades toward the
                  top-left corner (collapse) or back out (expand). Transforms
                  ONLY: the OS window stays full-size for the whole animation
                  and only snap-resizes at the endpoints (durationMs:0). The
                  ResizeObserver and the two chat useLayoutEffects are parked
                  via isMorphingRef/isMinimizedRef so no layout write fights
                  the transform. transformOrigin keeps the top-left pinned so
                  scaling alone visibly shrinks the card into the corner. */}
              <motion.div
                ref={setLayoutWrapperRef}
                animate={morphTransform}
                transition={morphTransition}
                onAnimationComplete={settleMorphPhase}
                style={{
                  transformOrigin: anchorToTransformOrigin(morphAnchor),
                }}
                className={
                  isSettledMinimized
                    ? ''
                    : `bg-surface-base backdrop-blur-2xl border border-surface-border ${
                        isChatMode
                          ? 'rounded-lg shadow-chat'
                          : 'rounded-2xl shadow-bar'
                      }`
                }
              >
                <AnimatePresence mode="wait">
                  {isSettledMinimized ? null : (
                    <motion.div
                      key="chat-content"
                      // While the chat is collapsing or expanding it is still
                      // mounted but scaled down toward the top-left corner,
                      // where the close (X) button sits — directly under the
                      // floating mascot, which is itself pointer-events-none
                      // mid-morph. Without this gate a click aimed at the
                      // mascot during a morph falls through it onto the close
                      // button and hides the whole overlay (the
                      // disappears-after-many-toggles bug). Disable pointer
                      // events on the entire chat subtree while morphing;
                      // it is fully interactive again once settled at idle.
                      className={isMorphing ? 'pointer-events-none' : undefined}
                      initial={{ opacity: 0 }}
                      animate={{ opacity: 1 }}
                      exit={{ opacity: 0, transition: { duration: 0.16 } }}
                      transition={{ duration: 0.12 }}
                    >
                      <>
                        {/* Morphing Container - flex column ensures the input bar
                    always sticks to the bottom. overflow-hidden clips chat
                    content during the morph animation. Visual styling lives on
                    the outer layout wrapper so the footer extends the window
                    without being clipped. */}
                        <div
                          ref={setContainerRef}
                          style={{
                            transition:
                              'height 0.25s cubic-bezier(0.16, 1, 0.3, 1), min-height 0.25s cubic-bezier(0.16, 1, 0.3, 1)',
                            maxHeight: `${config.window.maxChatHeight}px`,
                          }}
                          className="morphing-container relative flex flex-col overflow-hidden"
                        >
                          {/* Chat Messages Area - morphs in when in chat mode. */}
                          <AnimatePresence>
                            {isChatMode ? (
                              <ConversationView
                                messages={
                                  pendingUserMessage
                                    ? [...messages, pendingUserMessage]
                                    : messages
                                }
                                isGenerating={isGenerating || isSubmitPending}
                                onClose={handleCloseOverlay}
                                onSave={handleSave}
                                isSaved={isSaved}
                                canSave={canSave}
                                onNewConversation={handleNewConversation}
                                onHistoryOpen={handleHistoryToggle}
                                onImagePreview={handleChatImagePreview}
                                searchStage={searchStage}
                                activeModel={activeModel}
                                onModelPickerToggle={
                                  ollamaReachable
                                    ? handleModelPickerToggle
                                    : undefined
                                }
                                isModelPickerOpen={isModelPickerOpen}
                                onMinimize={handleMinimize}
                                onExportToggle={
                                  messages.length > 0
                                    ? handleExportToggle
                                    : undefined
                                }
                                isExportOpen={isExportOpen}
                              />
                            ) : null}
                          </AnimatePresence>

                          {/* Ask-bar mode model picker drawer - above the input bar.
                    In chat mode the trigger and drawer move to the header area above. */}
                          {!isChatMode && (
                            <AnimatePresence>
                              {isModelPickerOpen && ollamaReachable ? (
                                <motion.div
                                  ref={modelPickerAskBarRef}
                                  key="model-picker-askbar"
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
                                  <ModelPickerPanel
                                    models={availableModels}
                                    activeModel={activeModel}
                                    onSelect={handleModelSelect}
                                    onClose={handleModelPickerClose}
                                    capabilities={modelCapabilities}
                                  />
                                </motion.div>
                              ) : null}
                            </AnimatePresence>
                          )}

                          {/* Ask-bar mode history panel - inline below the input bar.
                    The !isChatMode gate lives OUTSIDE AnimatePresence so that when
                    a conversation is loaded (isChatMode → true) the panel unmounts
                    instantly - no exit animation runs alongside ConversationView
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
                                    onDeleteConversation={
                                      handleDeleteConversation
                                    }
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

                          {/* Input Bar - always pinned to the bottom */}
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
                            attachedImages={
                              isSubmitPending ? [] : attachedImages
                            }
                            onImagesAttached={handleImagesAttached}
                            onImageRemove={handleImageRemove}
                            onImagePreview={handleAskBarImagePreview}
                            onScreenshot={handleScreenshot}
                            isDragOver={isDragOver ?? undefined}
                            onModelPickerToggle={
                              ollamaReachable
                                ? handleModelPickerToggle
                                : undefined
                            }
                            isModelPickerOpen={isModelPickerOpen}
                            capabilityConflictMessage={
                              liveCapabilityConflictMessage
                            }
                            shake={shakeAskBar}
                            maxImages={config.window.maxImages}
                            onFirstKeystroke={() =>
                              void invoke('warm_up_model')
                            }
                          />
                        </div>

                        {/* Footer slot — outside the morphing container so overflow-hidden
                    never clips it when chat is at max height. The layout wrapper
                    provides the matching background, so both UpdateFooterBar and
                    TipBar render seamlessly below the conversation area.
                    UpdateFooterBar takes priority and renders in BOTH modes. */}
                        {showUpdate ? (
                          <AnimatePresence>
                            <motion.div
                              key="update-footer"
                              initial={{ height: 0, opacity: 0 }}
                              animate={{ height: 'auto', opacity: 1 }}
                              exit={{ height: 0, opacity: 0 }}
                              transition={{
                                duration: 0.25,
                                ease: [0.16, 1, 0.3, 1],
                              }}
                              style={{ overflow: 'hidden' }}
                            >
                              <UpdateFooterBar
                                version={updater.state.update!.version}
                                notesUrl={updater.state.update!.notes_url}
                                onInstall={() => void updater.openWindow()}
                                onLater={() => void updater.snoozeChat(24)}
                              />
                            </motion.div>
                          </AnimatePresence>
                        ) : (
                          <AnimatePresence>
                            {isTipVisible && (
                              <motion.div
                                key="tip-bar"
                                initial={{ height: 0, opacity: 0 }}
                                animate={{ height: 'auto', opacity: 1 }}
                                exit={{ height: 0, opacity: 0 }}
                                transition={{
                                  duration: 0.25,
                                  ease: [0.16, 1, 0.3, 1],
                                }}
                                style={{ overflow: 'hidden' }}
                              >
                                <TipBar
                                  tip={activeTip}
                                  tipKey={tipKey}
                                  skipAnimation={tipAlreadyAnimated}
                                />
                              </motion.div>
                            )}
                          </AnimatePresence>
                        )}
                      </>
                    </motion.div>
                  )}
                </AnimatePresence>
              </motion.div>

              {/* Morph mascot: portaled to <body> so it is anchored to the
                  viewport top-left (0,0) regardless of the transformed chat
                  ancestors. That is exactly where the native 48px window
                  snaps on collapse, so the icon and the window frame coincide
                  by construction and the icon can never be clipped/vanish.

                  Opacity/scale are driven by CSS (see `.thuki-mascot*` in
                  App.css), NOT Framer Motion. This is load-bearing: the panel
                  is a nonactivating NSPanel, and when it is not the key window
                  WKWebView throttles requestAnimationFrame. A Framer rAF tween
                  would then never repaint the mascot off its initial opacity:0
                  and the icon would vanish (it did, after ~7-8 rapid toggles).
                  A CSS-declared end-state is painted by the compositor
                  regardless of the rAF clock, so the settled icon is always
                  visible. Phase → class: `collapsing` blooms in, `minimized`
                  is the static visible base, `expanding` fades out. */}
              {isMinimized &&
                createPortal(
                  <div
                    key="morph-mascot"
                    className={`thuki-mascot fixed ${mascotCornerClass}${
                      isSettledMinimized ? '' : ' pointer-events-none'
                    }${morphPhase === 'collapsing' ? ' thuki-mascot-bloom' : ''}${
                      morphPhase === 'expanding' ? ' thuki-mascot-leaving' : ''
                    }`}
                    style={{
                      width: MINIMIZED_WINDOW_SIZE,
                      height: MINIMIZED_WINDOW_SIZE,
                    }}
                  >
                    <MinimizedIcon
                      isWorking={isGenerating}
                      hasUnseen={unseenCompletion}
                      onRestore={handleRestore}
                    />
                  </div>,
                  document.body,
                )}

              {/* Chat-mode model picker dropdown - floating card identical in style
                  to the chat-history dropdown. Anchored absolute right-3 top-10
                  so it appears just below the header pill trigger without pushing
                  the conversation content. Click-outside closes it. */}
              <AnimatePresence>
                {isChatMode && isModelPickerOpen && ollamaReachable ? (
                  <motion.div
                    ref={modelPickerDropdownRef}
                    key="model-picker-dropdown"
                    initial={{ opacity: 0, y: -8, scale: 0.97 }}
                    animate={{ opacity: 1, y: 0, scale: 1 }}
                    exit={{ opacity: 0, y: -8, scale: 0.97 }}
                    transition={{ type: 'spring', stiffness: 400, damping: 30 }}
                    className="absolute right-3 top-10 z-50 w-56 rounded-xl border border-surface-border bg-surface-base shadow-chat overflow-hidden flex flex-col"
                  >
                    <ModelPickerPanel
                      models={availableModels}
                      activeModel={activeModel}
                      onSelect={handleModelSelect}
                      onClose={handleModelPickerClose}
                      capabilities={modelCapabilities}
                      compact
                    />
                  </motion.div>
                ) : null}
              </AnimatePresence>

              {/* Chat-mode export popover. Anchored to the same right-3 top-10
                  corner as the model picker dropdown; the two never overlap
                  because opening one closes the other. Visual treatment mirrors
                  the `SwitchConfirmation` prompt (sentence-case title, plain
                  text rows, primary-highlighted recommended action) so the two
                  small popovers feel like a single language. */}
              <AnimatePresence>
                {isChatMode && isExportOpen ? (
                  <motion.div
                    ref={exportPopoverRef}
                    key="export-popover"
                    initial={{ opacity: 0, y: -8, scale: 0.97 }}
                    animate={{ opacity: 1, y: 0, scale: 1 }}
                    exit={{ opacity: 0, y: -8, scale: 0.97 }}
                    transition={{ type: 'spring', stiffness: 400, damping: 30 }}
                    role="dialog"
                    aria-label="Export chat"
                    className="absolute right-3 top-10 z-50 w-56 rounded-xl border border-surface-border bg-surface-base shadow-chat overflow-hidden"
                  >
                    <div className="px-3 py-3 flex flex-col gap-2.5">
                      <p className="text-xs text-text-secondary leading-snug">
                        Export chat
                      </p>
                      <div className="flex flex-col gap-1.5">
                        <button
                          type="button"
                          onClick={() => void runFileExport()}
                          className="w-full text-left px-3 py-2 rounded-lg text-xs text-text-primary hover:bg-white/5 transition-colors duration-150 cursor-pointer"
                        >
                          Save as Markdown…
                        </button>
                        <button
                          type="button"
                          onClick={() => void runClipboardCopy()}
                          className="w-full text-left px-3 py-2 rounded-lg text-xs text-text-primary hover:bg-white/5 transition-colors duration-150 cursor-pointer"
                        >
                          Copy to clipboard
                        </button>
                      </div>
                    </div>
                  </motion.div>
                ) : null}
              </AnimatePresence>

              {/* Chat-mode history dropdown - sibling of the morphing container so
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
