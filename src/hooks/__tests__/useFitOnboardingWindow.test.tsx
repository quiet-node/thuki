import { render } from '@testing-library/react';
import { useRef } from 'react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { useFitOnboardingWindow } from '../useFitOnboardingWindow';
import { invoke } from '../../testUtils/mocks/tauri';

/**
 * Renders the hook against a div whose measured box is stubbed to
 * `width`/`height` (jsdom never computes layout). When `width`/`height` are
 * undefined the node keeps its jsdom-default zero box; when `attach` is false
 * the ref is never pointed at a node.
 */
function Harness({
  width,
  height,
  attach = true,
  dep,
}: {
  width?: number;
  height?: number;
  attach?: boolean;
  dep?: unknown;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  useFitOnboardingWindow(ref, dep);
  return (
    <div
      data-testid="card"
      ref={(node) => {
        ref.current = attach ? node : null;
        if (node && width !== undefined && height !== undefined) {
          Object.defineProperty(node, 'offsetWidth', {
            configurable: true,
            value: width,
          });
          Object.defineProperty(node, 'offsetHeight', {
            configurable: true,
            value: height,
          });
        }
      }}
    />
  );
}

/** Resolves after the next animation frame, so a scheduled fit has run. */
const nextFrame = () =>
  new Promise((resolve) => requestAnimationFrame(() => resolve(undefined)));

/** Backend resize requests, filtered out from the alpha-reveal requests. */
const fitCalls = () =>
  invoke.mock.calls.filter((c) => c[0] === 'fit_onboarding_window');
/** Alpha-reveal requests the hook fires once the card size settles. */
const revealCalls = () =>
  invoke.mock.calls.filter((c) => c[0] === 'set_overlay_alpha');

describe('useFitOnboardingWindow', () => {
  beforeEach(() => {
    invoke.mockClear();
    invoke.mockResolvedValue(undefined);
  });

  it('asks the backend to fit and center the window to the measured box on spawn', async () => {
    render(<Harness width={474} height={612} />);
    await vi.waitFor(() => expect(fitCalls()).toHaveLength(1));

    expect(fitCalls()[0][1]).toEqual({
      width: 474,
      height: 612,
      center: true,
    });
  });

  it('does nothing when the card has no measured box', async () => {
    render(<Harness />);
    await nextFrame();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('does nothing when the ref is not attached', () => {
    render(<Harness width={474} height={612} attach={false} />);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('does nothing when only the height is unmeasured', async () => {
    render(<Harness width={474} height={0} />);
    await nextFrame();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('reveals the panel once the card size settles', async () => {
    render(<Harness width={474} height={612} />);
    // The fit lands first; the alpha reveal follows after the quiet window.
    await vi.waitFor(() => expect(fitCalls()).toHaveLength(1));
    await vi.waitFor(() => expect(revealCalls()).toHaveLength(1));
    expect(revealCalls()[0][1]).toEqual({ alpha: 1, durationMs: 150 });
  });

  it('keeps centering during the spawn settle window, then resizes in place', async () => {
    const now = vi.spyOn(Date, 'now');
    try {
      now.mockReturnValue(10_000);
      const { rerender } = render(<Harness width={474} height={612} dep={1} />);
      await vi.waitFor(() => expect(fitCalls()).toHaveLength(1));
      // A reflow inside the settle window still centers.
      expect(fitCalls()[0][1]).toEqual({
        width: 474,
        height: 612,
        center: true,
      });

      // Past the settle window: the re-fit resizes in place (center false), so a
      // later interaction or drag does not snap the window back to center.
      now.mockReturnValue(10_000 + 5_000);
      rerender(<Harness width={474} height={660} dep={2} />);
      await vi.waitFor(() => expect(fitCalls()).toHaveLength(2));
      expect(fitCalls()[1][1]).toEqual({
        width: 474,
        height: 660,
        center: false,
      });
    } finally {
      now.mockRestore();
    }
  });
});
