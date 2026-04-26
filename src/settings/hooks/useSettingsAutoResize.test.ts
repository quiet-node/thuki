import { renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useRef } from 'react';

import { __mockWindow } from '../../testUtils/mocks/tauri-window';
import { useSettingsAutoResize } from './useSettingsAutoResize';

const SETTINGS_WIDTH = 580;
const ANIMATE_MS = 220;
const MIN_HEIGHT = 280;
const MAX_HEIGHT = 900;
const CHROME = 148;

/**
 * Captures the latest ResizeObserver callback so tests can drive layout
 * changes manually. Mirrors the pattern in `App.test.tsx` since happy-dom
 * does not compute layout.
 */
let capturedRoCallback: ResizeObserverCallback | null = null;
function spyOnResizeObserver() {
  const Original = globalThis.ResizeObserver;
  vi.spyOn(globalThis, 'ResizeObserver').mockImplementation(function (
    cb: ResizeObserverCallback,
  ) {
    capturedRoCallback = cb;
    return new Original(cb) as ResizeObserver;
  });
}

function setScrollHeight(el: HTMLElement, h: number) {
  Object.defineProperty(el, 'scrollHeight', {
    configurable: true,
    value: h,
  });
}

function fireResize(el: HTMLElement, scrollHeight: number) {
  setScrollHeight(el, scrollHeight);
  capturedRoCallback?.(
    [{ target: el } as unknown as ResizeObserverEntry],
    {} as ResizeObserver,
  );
}

/** Strongly-typed view of `__mockWindow.setSize`'s recorded calls. */
function setSizeCalls(): Array<[{ width: number; height: number }]> {
  return __mockWindow.setSize.mock.calls as unknown as Array<
    [{ width: number; height: number }]
  >;
}

/**
 * Wrapper that gives us a real DOM element with a configurable initial
 * `scrollHeight` so the hook's first observation snap-call can be
 * tested with a known content size. The element identity is stable
 * across renders.
 */
function useHookWithEl(chromeHeight: number, initialScrollHeight = 400) {
  const ref = useRef<HTMLDivElement | null>(null);
  if (ref.current === null) {
    // eslint-disable-next-line @eslint-react/purity -- ref initializer for tests
    const el = document.createElement('div');
    setScrollHeight(el, initialScrollHeight);
    ref.current = el;
  }
  useSettingsAutoResize(ref, chromeHeight);
  return ref;
}

describe('useSettingsAutoResize', () => {
  beforeEach(() => {
    capturedRoCallback = null;
    __mockWindow.setSize.mockClear();
    spyOnResizeObserver();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('snaps without animation on the first measurement', () => {
    renderHook(() => useHookWithEl(CHROME, 400));
    // Hook fires `handleResize` once on mount.
    expect(__mockWindow.setSize).toHaveBeenCalledTimes(1);
    const call = setSizeCalls()[0][0];
    expect(call.width).toBe(SETTINGS_WIDTH);
    expect(call.height).toBe(400 + CHROME);
  });

  it('animates between sizes via requestAnimationFrame', () => {
    const { result } = renderHook(() => useHookWithEl(CHROME, 400));
    const el = result.current.current!;
    __mockWindow.setSize.mockClear();

    fireResize(el, 600);
    vi.advanceTimersByTime(ANIMATE_MS + 50);

    expect(setSizeCalls().length).toBeGreaterThan(1);
    const finalCall = setSizeCalls()[setSizeCalls().length - 1][0];
    expect(finalCall.height).toBe(600 + CHROME);
  });

  it('clamps to MAX_HEIGHT when content exceeds the cap', () => {
    renderHook(() => useHookWithEl(CHROME, 1200));
    const call = setSizeCalls()[0][0];
    expect(call.height).toBe(MAX_HEIGHT);
  });

  it('clamps to MIN_HEIGHT when content is too small', () => {
    renderHook(() => useHookWithEl(CHROME, 50));
    const call = setSizeCalls()[0][0];
    expect(call.height).toBe(MIN_HEIGHT);
  });

  it('skips negligible deltas (<4px)', () => {
    const { result } = renderHook(() => useHookWithEl(CHROME, 400));
    const el = result.current.current!;
    __mockWindow.setSize.mockClear();

    fireResize(el, 401);
    vi.advanceTimersByTime(ANIMATE_MS + 50);
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('cancels an in-flight animation when a new target arrives', () => {
    const { result } = renderHook(() => useHookWithEl(CHROME, 400));
    const el = result.current.current!;
    __mockWindow.setSize.mockClear();

    fireResize(el, 600);
    vi.advanceTimersByTime(60);
    const midCount = setSizeCalls().length;

    fireResize(el, 700);
    vi.advanceTimersByTime(ANIMATE_MS + 50);

    const finalCall = setSizeCalls()[setSizeCalls().length - 1][0];
    expect(finalCall.height).toBe(700 + CHROME);
    expect(setSizeCalls().length).toBeGreaterThan(midCount);
  });

  it('cleans up the observer and pending rAF on unmount', () => {
    const { result, unmount } = renderHook(() => useHookWithEl(CHROME, 400));
    const el = result.current.current!;
    fireResize(el, 600); // start animating
    unmount();
    __mockWindow.setSize.mockClear();
    vi.advanceTimersByTime(ANIMATE_MS + 50);
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('is a no-op when the ref has no element', () => {
    const { result } = renderHook(() => {
      const ref = useRef<HTMLDivElement | null>(null);
      useSettingsAutoResize(ref, CHROME);
      return ref;
    });
    expect(result.current.current).toBeNull();
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('reflects updated chromeHeight on the next resize', () => {
    let chrome = CHROME;
    const { result, rerender } = renderHook(() => {
      const ref = useRef<HTMLDivElement | null>(null);
      if (ref.current === null) {
        const el = document.createElement('div');
        setScrollHeight(el, 400);
        ref.current = el;
      }
      useSettingsAutoResize(ref, chrome);
      return ref;
    });
    const el = result.current.current!;
    expect(setSizeCalls()[0][0].height).toBe(400 + CHROME);

    chrome = CHROME + 56;
    rerender();
    __mockWindow.setSize.mockClear();
    fireResize(el, 400); // same content, new chrome
    vi.advanceTimersByTime(ANIMATE_MS + 50);
    const last = setSizeCalls()[setSizeCalls().length - 1][0];
    expect(last.height).toBe(400 + CHROME + 56);
  });
});
