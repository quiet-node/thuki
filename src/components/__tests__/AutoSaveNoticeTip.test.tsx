import { createRef } from 'react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, fireEvent, act } from '@testing-library/react';
import {
  AutoSaveNoticeTip,
  computeAutoSaveNoticePosition,
} from '../AutoSaveNoticeTip';
import { AUTO_SAVE_NOTICE_ANNOUNCEMENT } from '../../config/versionAnnouncements';
import { mockReducedMotion } from '../../testUtils/mocks/framer-motion';

/**
 * Stubs getBoundingClientRect on an element used as the bookmark anchor.
 */
function stubRect(
  el: HTMLElement,
  rect: Partial<DOMRect> & Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>,
): void {
  vi.spyOn(el, 'getBoundingClientRect').mockReturnValue({
    right: rect.left + rect.width,
    bottom: rect.top + rect.height,
    x: rect.left,
    y: rect.top,
    toJSON: () => ({}),
    ...rect,
  } as DOMRect);
}

describe('computeAutoSaveNoticePosition', () => {
  const originalInnerWidth = window.innerWidth;

  afterEach(() => {
    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: originalInnerWidth,
    });
  });

  it('right-aligns under a right-side bookmark and points caret at center', () => {
    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 600,
    });
    const anchor = document.createElement('button');
    stubRect(anchor, { left: 520, top: 10, width: 28, height: 28 });
    const coords = computeAutoSaveNoticePosition(anchor);
    // Prefer left = rect.right - 256 = 548 - 256 = 292
    expect(coords.left).toBe(292);
    expect(coords.top).toBe(10 + 28 + 8);
    // Center of bookmark = 520 + 14 = 534; offset from left = 534 - 292
    expect(coords.arrowOffset).toBe(534 - 292);
  });

  it('clamps left edge when preferred position would overflow left', () => {
    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 400,
    });
    const anchor = document.createElement('button');
    // Very left: preferred left = 40 - 256 = negative → clamp to EDGE_PADDING 8
    stubRect(anchor, { left: 12, top: 0, width: 28, height: 20 });
    const coords = computeAutoSaveNoticePosition(anchor);
    expect(coords.left).toBe(8);
    // Arrow clamped to stay on the card
    expect(coords.arrowOffset).toBeGreaterThanOrEqual(14);
    expect(coords.arrowOffset).toBeLessThanOrEqual(256 - 14);
  });

  it('clamps right edge when preferred position would overflow the viewport', () => {
    Object.defineProperty(window, 'innerWidth', {
      configurable: true,
      value: 400,
    });
    const anchor = document.createElement('button');
    // right = 408 → preferred left 152; maxLeft = 400 - 256 - 8 = 136
    stubRect(anchor, { left: 380, top: 5, width: 28, height: 20 });
    const coords = computeAutoSaveNoticePosition(anchor);
    expect(coords.left).toBe(136);
  });
});

describe('AutoSaveNoticeTip', () => {
  beforeEach(() => {
    mockReducedMotion.current = false;
  });

  afterEach(() => {
    mockReducedMotion.current = false;
  });

  it('renders nothing interactive when closed', () => {
    const anchorRef = createRef<HTMLButtonElement>();
    render(
      <>
        <button ref={anchorRef} type="button">
          Save
        </button>
        <AutoSaveNoticeTip
          open={false}
          anchorRef={anchorRef}
          onAcknowledge={vi.fn()}
          onOpenSettings={vi.fn()}
        />
      </>,
    );
    expect(screen.queryByTestId('auto-save-notice')).not.toBeInTheDocument();
  });

  it('renders title, body, caret, and fires action buttons when open', () => {
    const onAcknowledge = vi.fn();
    const onOpenSettings = vi.fn();
    const anchorRef = createRef<HTMLButtonElement>();
    render(
      <>
        <button ref={anchorRef} type="button">
          Save
        </button>
        <AutoSaveNoticeTip
          open
          anchorRef={anchorRef}
          onAcknowledge={onAcknowledge}
          onOpenSettings={onOpenSettings}
        />
      </>,
    );
    if (anchorRef.current) {
      stubRect(anchorRef.current, {
        left: 400,
        top: 8,
        width: 28,
        height: 28,
      });
    }
    act(() => {
      fireEvent(window, new Event('resize'));
    });

    const tip = screen.getByTestId('auto-save-notice');
    expect(tip).toBeInTheDocument();
    expect(tip).toHaveAttribute('role', 'region');
    expect(tip).toHaveAttribute(
      'aria-label',
      AUTO_SAVE_NOTICE_ANNOUNCEMENT.title,
    );
    expect(
      screen.getByText(AUTO_SAVE_NOTICE_ANNOUNCEMENT.title),
    ).toBeInTheDocument();
    expect(
      screen.getByText(AUTO_SAVE_NOTICE_ANNOUNCEMENT.body),
    ).toBeInTheDocument();
    expect(screen.getByTestId('auto-save-notice-caret')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Acknowledge' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Turn off in Settings' }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('auto-save-notice-ack'));
    expect(onAcknowledge).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByTestId('auto-save-notice-settings'));
    expect(onOpenSettings).toHaveBeenCalledOnce();
  });

  it('repositions on window scroll while open', () => {
    const anchorRef = createRef<HTMLButtonElement>();
    render(
      <>
        <button ref={anchorRef} type="button">
          Save
        </button>
        <AutoSaveNoticeTip
          open
          anchorRef={anchorRef}
          onAcknowledge={vi.fn()}
          onOpenSettings={vi.fn()}
        />
      </>,
    );
    if (anchorRef.current) {
      stubRect(anchorRef.current, {
        left: 100,
        top: 40,
        width: 28,
        height: 28,
      });
    }
    act(() => {
      fireEvent(window, new Event('scroll'));
    });
    expect(screen.getByTestId('auto-save-notice')).toBeInTheDocument();
  });

  it('uses opacity-only motion under prefers-reduced-motion', () => {
    mockReducedMotion.current = true;
    const anchorRef = createRef<HTMLButtonElement>();
    render(
      <>
        <button ref={anchorRef} type="button">
          Save
        </button>
        <AutoSaveNoticeTip
          open
          anchorRef={anchorRef}
          onAcknowledge={vi.fn()}
          onOpenSettings={vi.fn()}
        />
      </>,
    );
    expect(screen.getByTestId('auto-save-notice')).toBeInTheDocument();
  });

  it('honors a custom testId prefix for root and actions', () => {
    const anchorRef = createRef<HTMLButtonElement>();
    render(
      <>
        <button ref={anchorRef} type="button">
          Save
        </button>
        <AutoSaveNoticeTip
          open
          anchorRef={anchorRef}
          onAcknowledge={vi.fn()}
          onOpenSettings={vi.fn()}
          testId="custom-notice"
        />
      </>,
    );
    expect(screen.getByTestId('custom-notice')).toBeInTheDocument();
    expect(screen.getByTestId('custom-notice-caret')).toBeInTheDocument();
    expect(screen.getByTestId('custom-notice-ack')).toBeInTheDocument();
    expect(screen.getByTestId('custom-notice-settings')).toBeInTheDocument();
  });
});
