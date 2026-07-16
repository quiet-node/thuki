/**
 * Compact floating auto-save notice under the title-bar bookmark.
 *
 * Design C: glass tip card with top caret centered on the bookmark button.
 * Portaled to `document.body` so it floats over chat without layout thrash.
 * Host owns open state and dismiss / Settings deep-link persistence.
 */

import { AnimatePresence, motion, useReducedMotion } from 'framer-motion';
import { useLayoutEffect, useState, type RefObject } from 'react';
import { createPortal } from 'react-dom';
import {
  AUTO_SAVE_NOTICE_ANNOUNCEMENT,
  autoSaveNoticeSettingsCta,
} from '../config/versionAnnouncements';

/** House decelerate curve shared with ask-bar and Settings surfaces. */
const HOUSE_EASE: [number, number, number, number] = [0.16, 1, 0.3, 1];

/** Compact card max width (design C: ~240–268px). */
const TIP_MAX_WIDTH = 256;

/** Gap between bookmark bottom edge and tip top edge. */
const ANCHOR_GAP = 8;

/** Viewport edge padding when clamping horizontal position. */
const EDGE_PADDING = 8;

/** Keep caret inset from card edges so the triangle stays on the surface. */
const ARROW_EDGE_PAD = 14;

/** Outlined primary CTA, matching VersionAnnouncement / SearchTrustNotice. */
const PRIMARY_BTN_CLASS =
  'cursor-pointer rounded-lg border border-primary/45 bg-transparent px-3 py-1.5 text-[11.5px] font-semibold text-primary transition-colors hover:bg-primary/10';

/** Ghost secondary CTA. */
const GHOST_BTN_CLASS =
  'cursor-pointer border-0 bg-transparent px-1 py-1.5 text-[11.5px] font-medium text-white/50 transition-colors hover:text-white/75';

export interface AutoSaveNoticeTipProps {
  /** When true, measure the anchor and show the tip. */
  open: boolean;
  /** Bookmark button (or wrapper) to position under. */
  anchorRef: RefObject<HTMLElement | null>;
  /** Persist dismiss and hide the tip. */
  onAcknowledge: () => void;
  /** Open Settings › Behavior with Auto-save highlighted. */
  onOpenSettings: () => void;
  /** Root data-testid; default `auto-save-notice`. */
  testId?: string;
}

/**
 * Fixed-position coords for the tip card relative to the viewport.
 *
 * `left` is the card's left edge (clamped). `arrowOffset` is the bookmark
 * center X relative to that left edge so the caret tracks the button even
 * when the card is nudged away from a window edge.
 */
interface TipCoords {
  left: number;
  top: number;
  arrowOffset: number;
}

/**
 * Computes tip placement under `anchor` with viewport clamping.
 *
 * Prefers right-aligning the card toward the bookmark cluster (typical
 * right-side title-bar control), then clamps so the card stays on screen.
 *
 * @param anchor Element to center the caret on (bookmark button).
 * @returns Fixed left/top and caret offset from the card's left edge.
 */
export function computeAutoSaveNoticePosition(anchor: HTMLElement): TipCoords {
  const rect = anchor.getBoundingClientRect();
  const anchorCenterX = rect.left + rect.width / 2;
  // Prefer right-align under the bookmark so a right-side control does not
  // shove the card into the left half of a narrow overlay.
  let left = rect.right - TIP_MAX_WIDTH;
  const maxLeft = window.innerWidth - TIP_MAX_WIDTH - EDGE_PADDING;
  left = Math.max(EDGE_PADDING, Math.min(left, maxLeft));
  const top = rect.bottom + ANCHOR_GAP;
  const rawArrow = anchorCenterX - left;
  const arrowOffset = Math.max(
    ARROW_EDGE_PAD,
    Math.min(TIP_MAX_WIDTH - ARROW_EDGE_PAD, rawArrow),
  );
  return { left, top, arrowOffset };
}

/**
 * Floating tip announcing conversation auto-save under the bookmark.
 *
 * @param open Whether the tip is visible.
 * @param anchorRef Bookmark button used for position + caret.
 * @param onAcknowledge Acknowledge dismiss handler.
 * @param onOpenSettings Settings deep-link handler (Turn off in Settings).
 * @param testId Root test id.
 */
export function AutoSaveNoticeTip({
  open,
  anchorRef,
  onAcknowledge,
  onOpenSettings,
  testId = 'auto-save-notice',
}: AutoSaveNoticeTipProps) {
  const reduceMotion = useReducedMotion();
  const [coords, setCoords] = useState<TipCoords>({
    left: 0,
    top: 0,
    arrowOffset: TIP_MAX_WIDTH / 2,
  });

  useLayoutEffect(() => {
    if (!open) return;

    /**
     * Re-reads the bookmark rect and updates fixed coords.
     * No-ops when the anchor is unmounted (e.g. save button hidden).
     */
    const updatePosition = (): void => {
      const el = anchorRef.current;
      /* v8 ignore start */
      if (!el) return;
      /* v8 ignore stop */
      // eslint-disable-next-line @eslint-react/set-state-in-effect -- DOM measure → tip coords
      setCoords(computeAutoSaveNoticePosition(el));
    };

    // Measure after layout so the caret tracks the live bookmark rect.
    updatePosition();
    window.addEventListener('resize', updatePosition);
    // Capture scroll from nested chat containers as well as the window.
    window.addEventListener('scroll', updatePosition, true);
    return () => {
      window.removeEventListener('resize', updatePosition);
      window.removeEventListener('scroll', updatePosition, true);
    };
  }, [open, anchorRef]);

  const title = AUTO_SAVE_NOTICE_ANNOUNCEMENT.title;
  const body = AUTO_SAVE_NOTICE_ANNOUNCEMENT.body;

  const enterMotion = reduceMotion
    ? { opacity: 0 }
    : { opacity: 0, y: -4, scale: 0.98 };
  const shownMotion = reduceMotion
    ? { opacity: 1 }
    : { opacity: 1, y: 0, scale: 1 };
  const exitMotion = reduceMotion
    ? { opacity: 0, transition: { duration: 0.01 } }
    : {
        opacity: 0,
        y: -4,
        scale: 0.98,
        transition: { duration: 0.16, ease: 'easeIn' as const },
      };
  const enterTransition = reduceMotion
    ? { duration: 0.01 }
    : { duration: 0.18, ease: HOUSE_EASE };

  return createPortal(
    <AnimatePresence>
      {open ? (
        <div
          style={{
            position: 'fixed',
            left: coords.left,
            top: coords.top,
            width: TIP_MAX_WIDTH,
            maxWidth: `min(${TIP_MAX_WIDTH}px, calc(100vw - ${EDGE_PADDING * 2}px))`,
            zIndex: 9998,
          }}
        >
          <motion.div
            data-testid={testId}
            role="region"
            aria-label={title}
            initial={enterMotion}
            animate={shownMotion}
            exit={exitMotion}
            transition={enterTransition}
            className="relative rounded-xl border border-surface-border bg-surface-base/95 px-3.5 py-3 shadow-chat backdrop-blur-2xl"
          >
            {/* Top caret: tip points at bookmark center via arrowOffset. */}
            <div
              aria-hidden="true"
              data-testid={`${testId}-caret`}
              style={{
                left: coords.arrowOffset,
              }}
              className="absolute -top-1.5 h-3 w-3 -translate-x-1/2 rotate-45 border-l border-t border-surface-border bg-surface-base/95"
            />
            <p className="relative text-[13px] font-medium text-white/90 leading-snug">
              {title}
            </p>
            <p className="relative mt-1 text-xs text-white/45 leading-relaxed">
              {body}
            </p>
            <div className="relative mt-2.5 flex flex-wrap items-center gap-2">
              <button
                type="button"
                data-testid={`${testId}-ack`}
                onClick={onAcknowledge}
                className={PRIMARY_BTN_CLASS}
              >
                Acknowledge
              </button>
              <button
                type="button"
                data-testid={`${testId}-settings`}
                onClick={onOpenSettings}
                className={GHOST_BTN_CLASS}
              >
                {autoSaveNoticeSettingsCta()}
              </button>
            </div>
          </motion.div>
        </div>
      ) : null}
    </AnimatePresence>,
    document.body,
  );
}
