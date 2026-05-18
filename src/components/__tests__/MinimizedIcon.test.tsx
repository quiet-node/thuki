import { render, screen, fireEvent } from '@testing-library/react';
import { beforeEach, describe, it, expect, vi } from 'vitest';
import { __mockWindow } from '../../testUtils/mocks/tauri-window';
import { MinimizedIcon } from '../MinimizedIcon';

describe('MinimizedIcon', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('calls onRestore on a plain click (no drag)', () => {
    const onRestore = vi.fn();
    render(
      <MinimizedIcon
        isWorking={false}
        hasUnseen={false}
        onRestore={onRestore}
      />,
    );
    const btn = screen.getByRole('button', { name: /restore thuki/i });
    fireEvent.pointerDown(btn, { clientX: 0, clientY: 0 });
    fireEvent.pointerUp(btn, { clientX: 1, clientY: 1 });
    expect(onRestore).toHaveBeenCalledTimes(1);
  });

  it('does not call onRestore when the pointer moves past the drag threshold', () => {
    const onRestore = vi.fn();
    render(
      <MinimizedIcon
        isWorking={false}
        hasUnseen={false}
        onRestore={onRestore}
      />,
    );
    const btn = screen.getByRole('button', { name: /restore thuki/i });
    fireEvent.pointerDown(btn, { clientX: 0, clientY: 0 });
    fireEvent.pointerMove(btn, { clientX: 40, clientY: 40 });
    fireEvent.pointerUp(btn, { clientX: 40, clientY: 40 });
    expect(onRestore).not.toHaveBeenCalled();
  });

  it('starts the native drag when moved past the threshold', () => {
    render(
      <MinimizedIcon isWorking={false} hasUnseen={false} onRestore={vi.fn()} />,
    );
    const btn = screen.getByRole('button', { name: /restore thuki/i });
    fireEvent.pointerDown(btn, { clientX: 0, clientY: 0 });
    fireEvent.pointerMove(btn, { clientX: 40, clientY: 40 });
    expect(__mockWindow.startDragging).toHaveBeenCalled();
  });

  it('ignores pointermove with no prior pointerdown', () => {
    render(
      <MinimizedIcon isWorking={false} hasUnseen={false} onRestore={vi.fn()} />,
    );
    const btn = screen.getByRole('button', { name: /restore thuki/i });
    fireEvent.pointerMove(btn, { clientX: 40, clientY: 40 });
    expect(__mockWindow.startDragging).not.toHaveBeenCalled();
  });

  it('shows the working state when isWorking is true', () => {
    render(<MinimizedIcon isWorking hasUnseen={false} onRestore={vi.fn()} />);
    expect(screen.getByTestId('minimized-working')).toBeInTheDocument();
  });

  it('does not show the working state when isWorking is false', () => {
    render(
      <MinimizedIcon isWorking={false} hasUnseen={false} onRestore={vi.fn()} />,
    );
    expect(screen.queryByTestId('minimized-working')).not.toBeInTheDocument();
  });

  it('shows the ready dot when hasUnseen is true', () => {
    render(<MinimizedIcon isWorking={false} hasUnseen onRestore={vi.fn()} />);
    expect(screen.getByTestId('minimized-ready-dot')).toBeInTheDocument();
  });

  it('does not show the ready dot when hasUnseen is false', () => {
    render(
      <MinimizedIcon isWorking={false} hasUnseen={false} onRestore={vi.fn()} />,
    );
    expect(screen.queryByTestId('minimized-ready-dot')).not.toBeInTheDocument();
  });

  it('does not re-start drag on subsequent moves past threshold', () => {
    render(
      <MinimizedIcon isWorking={false} hasUnseen={false} onRestore={vi.fn()} />,
    );
    const btn = screen.getByRole('button', { name: /restore thuki/i });
    fireEvent.pointerDown(btn, { clientX: 0, clientY: 0 });
    fireEvent.pointerMove(btn, { clientX: 40, clientY: 40 });
    fireEvent.pointerMove(btn, { clientX: 80, clientY: 80 });
    expect(__mockWindow.startDragging).toHaveBeenCalledTimes(1);
  });
});
