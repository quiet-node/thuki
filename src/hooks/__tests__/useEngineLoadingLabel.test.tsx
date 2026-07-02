import { renderHook, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { useEngineLoadingLabel } from '../useEngineLoadingLabel';
import {
  ENGINE_LOADING_THRESHOLD_MS,
  ENGINE_PHASE1_PHRASES,
  ENGINE_PHASE1_INTERVAL_MS,
  ENGINE_PHASE2_PHRASES,
  ENGINE_PHASE2_INTERVAL_MS,
  ENGINE_SLOW_WARM_LABEL,
} from '../../config/engineLoadingLabels';

describe('useEngineLoadingLabel', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders no label when inactive', () => {
    const { result } = renderHook(() =>
      useEngineLoadingLabel(false, 'builtin', false, 'stopped'),
    );
    expect(result.current).toBeNull();
  });

  it('renders no label for a remote provider', () => {
    const { result } = renderHook(() =>
      useEngineLoadingLabel(true, 'openai', false, 'stopped'),
    );
    act(() => {
      vi.advanceTimersByTime(10000);
    });
    expect(result.current).toBeNull();
  });

  it('stays null before the threshold elapses (fast/warm turn)', () => {
    const { result } = renderHook(() =>
      useEngineLoadingLabel(true, 'builtin', false, 'starting'),
    );
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS - 1);
    });
    expect(result.current).toBeNull();
  });

  it('shows the first phase-1 phrase once the threshold elapses', () => {
    const { result } = renderHook(() =>
      useEngineLoadingLabel(true, 'builtin', false, 'starting'),
    );
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[0]);
  });

  it('steps to the second phase-1 phrase at the configured interval', () => {
    const { result } = renderHook(() =>
      useEngineLoadingLabel(true, 'ollama', false, 'starting'),
    );
    act(() => {
      vi.advanceTimersByTime(
        ENGINE_LOADING_THRESHOLD_MS + ENGINE_PHASE1_INTERVAL_MS,
      );
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[1]);
  });

  it('holds on the last phase-1 phrase for Ollama (no phase-2 signal exists)', () => {
    const { result } = renderHook(() =>
      useEngineLoadingLabel(true, 'ollama', false, 'starting'),
    );
    act(() => {
      vi.advanceTimersByTime(
        ENGINE_LOADING_THRESHOLD_MS + ENGINE_PHASE1_INTERVAL_MS * 10,
      );
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[1]);
  });

  it('jumps to the first phase-2 phrase immediately when warming fires', () => {
    const { result, rerender } = renderHook(
      ({ warming }) =>
        useEngineLoadingLabel(true, 'builtin', warming, 'starting'),
      { initialProps: { warming: false } },
    );
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[0]);

    rerender({ warming: true });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[0]);
  });

  it('cuts phase 1 short when warming fires before its second phrase', () => {
    const { result, rerender } = renderHook(
      ({ warming }) =>
        useEngineLoadingLabel(true, 'builtin', warming, 'starting'),
      { initialProps: { warming: false } },
    );
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[0]);

    rerender({ warming: true });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[0]);

    // The cancelled phase-1 timer must not fire later and clobber phase 2.
    act(() => {
      vi.advanceTimersByTime(ENGINE_PHASE1_INTERVAL_MS * 5);
    });
    expect(result.current).not.toBe(ENGINE_PHASE1_PHRASES[1]);
  });

  it('steps to the second phase-2 phrase once warming has run long enough', () => {
    const { result, rerender } = renderHook(
      ({ warming }) =>
        useEngineLoadingLabel(true, 'builtin', warming, 'starting'),
      { initialProps: { warming: true } },
    );
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[0]);

    act(() => {
      vi.advanceTimersByTime(ENGINE_PHASE2_INTERVAL_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[1]);

    // Confirm warming flipping false mid-phase-2 doesn't disturb the timer.
    rerender({ warming: false });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[1]);
  });

  it('does not regress to a phase-1 phrase after warming has fired, even if warming flips back false while still active', () => {
    // Reproduces a real bug: the built-in engine's prime can finish
    // (warmup:builtin-warmed) before the actual chat request's first token
    // arrives, so `warming` goes true -> false while `active` is still true.
    // The label must not fall back to a phase-1 phrase - that would
    // misreport a loaded model as still spinning up.
    const { result, rerender } = renderHook(
      ({ warming }) =>
        useEngineLoadingLabel(true, 'builtin', warming, 'starting'),
      { initialProps: { warming: false } },
    );
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[0]);

    rerender({ warming: true });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[0]);

    rerender({ warming: false });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[0]);

    act(() => {
      vi.advanceTimersByTime(ENGINE_PHASE2_INTERVAL_MS * 5);
    });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[1]);
  });

  it('is a no-op when warming flips true again while already in phase 2', () => {
    const { result, rerender } = renderHook(
      ({ warming }) =>
        useEngineLoadingLabel(true, 'builtin', warming, 'starting'),
      { initialProps: { warming: true } },
    );
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[0]);

    act(() => {
      vi.advanceTimersByTime(ENGINE_PHASE2_INTERVAL_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[1]);

    // Re-entering phase 2 (warming false -> true again) must not reset the
    // already-advanced phase-2 phrase back to its first phrase.
    rerender({ warming: false });
    rerender({ warming: true });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[1]);
  });

  it('never enters phase 2 for Ollama (no real signal exists)', () => {
    const { result } = renderHook(() =>
      useEngineLoadingLabel(true, 'ollama', false, 'starting'),
    );
    act(() => {
      vi.advanceTimersByTime(
        ENGINE_LOADING_THRESHOLD_MS + ENGINE_PHASE1_INTERVAL_MS * 10,
      );
    });
    expect(ENGINE_PHASE2_PHRASES).not.toContain(result.current);
  });

  it('clears the label once the turn becomes inactive', () => {
    const { result, rerender } = renderHook(
      ({ active }) =>
        useEngineLoadingLabel(active, 'builtin', false, 'starting'),
      { initialProps: { active: true } },
    );
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[0]);

    rerender({ active: false });
    expect(result.current).toBeNull();
  });

  it('restarts the threshold from zero on a fresh active turn', () => {
    const { result, rerender } = renderHook(
      ({ active }) =>
        useEngineLoadingLabel(active, 'builtin', false, 'starting'),
      { initialProps: { active: true } },
    );
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[0]);

    rerender({ active: false });
    rerender({ active: true });
    // A fresh turn must not inherit the previous turn's elapsed time.
    expect(result.current).toBeNull();
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS - 1);
    });
    expect(result.current).toBeNull();
  });

  it('does not carry the phase-2 latch over into a fresh turn', () => {
    const { result, rerender } = renderHook(
      ({ active, warming }) =>
        useEngineLoadingLabel(active, 'builtin', warming, 'starting'),
      { initialProps: { active: true, warming: false } },
    );
    rerender({ active: true, warming: true });
    expect(result.current).toBe(ENGINE_PHASE2_PHRASES[0]);

    rerender({ active: false, warming: false });
    expect(result.current).toBeNull();

    rerender({ active: true, warming: false });
    // A fresh cold start after the previous turn warmed must not inherit
    // that latch and jump straight back to phase 2.
    expect(result.current).toBeNull();
    act(() => {
      vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
    });
    expect(result.current).toBe(ENGINE_PHASE1_PHRASES[0]);
  });

  describe('engine already loaded (slow-warm path)', () => {
    it('never shows a phase-1 phrase when the engine is already loaded', () => {
      const { result } = renderHook(() =>
        useEngineLoadingLabel(true, 'builtin', false, 'loaded'),
      );
      act(() => {
        vi.advanceTimersByTime(
          ENGINE_LOADING_THRESHOLD_MS + ENGINE_PHASE1_INTERVAL_MS * 5,
        );
      });
      expect(ENGINE_PHASE1_PHRASES).not.toContain(result.current);
    });

    it('shows the slow-warm label once the threshold elapses on an already-loaded engine', () => {
      const { result } = renderHook(() =>
        useEngineLoadingLabel(true, 'builtin', false, 'loaded'),
      );
      act(() => {
        vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
      });
      expect(result.current).toBe(ENGINE_SLOW_WARM_LABEL);
    });

    it('stays null before the threshold elapses on an already-loaded engine', () => {
      const { result } = renderHook(() =>
        useEngineLoadingLabel(true, 'builtin', false, 'loaded'),
      );
      act(() => {
        vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS - 1);
      });
      expect(result.current).toBeNull();
    });

    it('holds on the slow-warm label indefinitely (no further rotation)', () => {
      const { result } = renderHook(() =>
        useEngineLoadingLabel(true, 'builtin', false, 'loaded'),
      );
      act(() => {
        vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS + 30000);
      });
      expect(result.current).toBe(ENGINE_SLOW_WARM_LABEL);
    });

    it('still jumps to phase 2 if warming fires on an already-loaded engine', () => {
      // Real world: the engine is loaded, and a fresh proactive prime kicks
      // off for this exact turn (e.g. a model switch just completed). The
      // real signal always wins over the slow-warm guess.
      const { result, rerender } = renderHook(
        ({ warming }) =>
          useEngineLoadingLabel(true, 'builtin', warming, 'loaded'),
        { initialProps: { warming: false } },
      );
      act(() => {
        vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
      });
      expect(result.current).toBe(ENGINE_SLOW_WARM_LABEL);

      rerender({ warming: true });
      expect(result.current).toBe(ENGINE_PHASE2_PHRASES[0]);
    });

    it('ignores a stray engineState=loaded for Ollama (engineState only ever describes the built-in engine)', () => {
      // A quirk of Ollama being the active provider: `engineState` still
      // reflects the built-in engine's own runner, so a value of "loaded"
      // here says nothing about Ollama's residency. Ollama must always use
      // the phase-1 cold-start filler, never the slow-warm skip.
      const { result } = renderHook(() =>
        useEngineLoadingLabel(true, 'ollama', false, 'loaded'),
      );
      act(() => {
        vi.advanceTimersByTime(ENGINE_LOADING_THRESHOLD_MS);
      });
      expect(result.current).toBe(ENGINE_PHASE1_PHRASES[0]);
    });
  });
});
