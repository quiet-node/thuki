import { renderHook, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useState } from 'react';

import { __mockWindow } from '../../testUtils/mocks/tauri-window';
import { useSettingsAutoResize } from './useSettingsAutoResize';

const SETTINGS_WIDTH = 580;
const ANIMATE_MS = 220;
const MIN_HEIGHT = 280;
const MAX_HEIGHT = 700;
const CHROME = 148;

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

function setSizeCalls(): Array<[{ width: number; height: number }]> {
  return __mockWindow.setSize.mock.calls as unknown as Array<
    [{ width: number; height: number }]
  >;
}

/**
 * Mounts the hook with a real DOM element. Mirrors how SettingsWindow
 * wires the state-backed callback ref, so the effect's `el`-dependent
 * setup path is exercised end-to-end.
 */
function makeHookHarness(initialScrollHeight = 400) {
  let updateRevision: ((rev: unknown) => void) | undefined;
  let updateChrome: ((c: number) => void) | undefined;
  const { result, rerender, unmount } = renderHook(() => {
    const [el, setEl] = useState<HTMLDivElement | null>(null);
    const [revision, setRevision] = useState<unknown>('initial');
    const [chrome, setChrome] = useState(CHROME);
    updateRevision = setRevision;
    updateChrome = setChrome;
    useSettingsAutoResize(el, chrome, revision);
    return { el, setEl };
  });

  // Simulate ref-callback firing with a freshly created element.
  const node = document.createElement('div');
  setScrollHeight(node, initialScrollHeight);
  act(() => {
    result.current.setEl(node);
  });

  return {
    el: node,
    setRevision: (r: unknown) =>
      act(() => {
        updateRevision!(r);
      }),
    setChrome: (c: number) =>
      act(() => {
        updateChrome!(c);
      }),
    rerender,
    unmount,
  };
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
    makeHookHarness(400);
    expect(__mockWindow.setSize).toHaveBeenCalledTimes(1);
    const call = setSizeCalls()[0][0];
    expect(call.width).toBe(SETTINGS_WIDTH);
    expect(call.height).toBe(400 + CHROME);
  });

  it('animates between sizes via requestAnimationFrame', () => {
    const { el } = makeHookHarness(300);
    __mockWindow.setSize.mockClear();

    fireResize(el, 500);
    vi.advanceTimersByTime(ANIMATE_MS + 50);

    expect(setSizeCalls().length).toBeGreaterThan(1);
    const finalCall = setSizeCalls()[setSizeCalls().length - 1][0];
    expect(finalCall.height).toBe(500 + CHROME);
  });

  it('clamps to MAX_HEIGHT when content exceeds the cap', () => {
    makeHookHarness(1200);
    const call = setSizeCalls()[0][0];
    expect(call.height).toBe(MAX_HEIGHT);
  });

  it('returns true when natural content exceeds MAX_HEIGHT', () => {
    const { result } = renderHook(() => {
      const [el, setEl] = useState<HTMLDivElement | null>(null);
      const clamped = useSettingsAutoResize(el, CHROME, 0);
      return { clamped, setEl };
    });
    const node = document.createElement('div');
    setScrollHeight(node, 1200);
    act(() => {
      result.current.setEl(node);
    });
    expect(result.current.clamped).toBe(true);
  });

  it('returns false when natural content fits under MAX_HEIGHT', () => {
    const { result } = renderHook(() => {
      const [el, setEl] = useState<HTMLDivElement | null>(null);
      const clamped = useSettingsAutoResize(el, CHROME, 0);
      return { clamped, setEl };
    });
    const node = document.createElement('div');
    setScrollHeight(node, 300);
    act(() => {
      result.current.setEl(node);
    });
    expect(result.current.clamped).toBe(false);
  });

  it('clamps to MIN_HEIGHT when content is too small', () => {
    makeHookHarness(50);
    const call = setSizeCalls()[0][0];
    expect(call.height).toBe(MIN_HEIGHT);
  });

  it('skips negligible deltas (<4px)', () => {
    const { el } = makeHookHarness(400);
    __mockWindow.setSize.mockClear();

    fireResize(el, 401);
    vi.advanceTimersByTime(ANIMATE_MS + 50);
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('cancels an in-flight animation when a new target arrives', () => {
    const { el } = makeHookHarness(300);
    __mockWindow.setSize.mockClear();

    fireResize(el, 400);
    vi.advanceTimersByTime(60);
    const midCount = setSizeCalls().length;

    fireResize(el, 500);
    vi.advanceTimersByTime(ANIMATE_MS + 50);

    const finalCall = setSizeCalls()[setSizeCalls().length - 1][0];
    expect(finalCall.height).toBe(500 + CHROME);
    expect(setSizeCalls().length).toBeGreaterThan(midCount);
  });

  it('cleans up the observer and pending rAF on unmount', () => {
    const { el, unmount } = makeHookHarness(300);
    fireResize(el, 500);
    unmount();
    __mockWindow.setSize.mockClear();
    vi.advanceTimersByTime(ANIMATE_MS + 50);
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('is a no-op while the element ref has not yet attached', () => {
    renderHook(() => {
      const [el] = useState<HTMLDivElement | null>(null);
      useSettingsAutoResize(el, CHROME, 0);
    });
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('forces a re-measure when the revision value changes (tab switch)', () => {
    const { el, setRevision } = makeHookHarness(300);
    __mockWindow.setSize.mockClear();

    setScrollHeight(el, 500);
    setRevision('next');
    vi.advanceTimersByTime(ANIMATE_MS + 50);

    const last = setSizeCalls()[setSizeCalls().length - 1][0];
    expect(last.height).toBe(500 + CHROME);
  });

  it('reflects updated chromeHeight on the next resize', () => {
    const { el, setChrome } = makeHookHarness(400);
    expect(setSizeCalls()[0][0].height).toBe(400 + CHROME);

    setChrome(CHROME + 56);
    __mockWindow.setSize.mockClear();
    fireResize(el, 400);
    vi.advanceTimersByTime(ANIMATE_MS + 50);
    const last = setSizeCalls()[setSizeCalls().length - 1][0];
    expect(last.height).toBe(400 + CHROME + 56);
  });
});
