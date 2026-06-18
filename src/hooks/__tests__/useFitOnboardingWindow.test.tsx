import { render } from '@testing-library/react';
import { useRef } from 'react';
import { describe, expect, it, vi, beforeEach } from 'vitest';
import { useFitOnboardingWindow } from '../useFitOnboardingWindow';
import { __mockWindow } from '../../testUtils/mocks/tauri-window';

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

describe('useFitOnboardingWindow', () => {
  beforeEach(() => {
    __mockWindow.setSize.mockClear();
    __mockWindow.center.mockClear();
  });

  it('sizes the window to the measured card box and re-centers', async () => {
    render(<Harness width={474} height={612} />);
    await vi.waitFor(() => expect(__mockWindow.center).toHaveBeenCalled());

    expect(__mockWindow.setSize).toHaveBeenCalledWith(
      expect.objectContaining({ width: 474, height: 612 }),
    );
    expect(__mockWindow.setSize).toHaveBeenCalledTimes(1);
  });

  it('does nothing when the card has no measured box', () => {
    render(<Harness />);
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('does nothing when the ref is not attached', () => {
    render(<Harness width={474} height={612} attach={false} />);
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('does nothing when only the height is unmeasured', () => {
    render(<Harness width={474} height={0} />);
    expect(__mockWindow.setSize).not.toHaveBeenCalled();
  });

  it('re-fits when a dependency changes (the strip grows the card)', async () => {
    const { rerender } = render(<Harness width={474} height={612} dep={1} />);
    await vi.waitFor(() =>
      expect(__mockWindow.setSize).toHaveBeenCalledTimes(1),
    );

    rerender(<Harness width={474} height={660} dep={2} />);
    await vi.waitFor(() =>
      expect(__mockWindow.setSize).toHaveBeenCalledTimes(2),
    );
    expect(__mockWindow.setSize).toHaveBeenLastCalledWith(
      expect.objectContaining({ height: 660 }),
    );
  });
});
