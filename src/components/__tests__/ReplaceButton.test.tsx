import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ReplaceButton } from '../ReplaceButton';

const LABEL = 'Replace selection in source app';

describe('ReplaceButton', () => {
  it('renders an accessible button', () => {
    render(
      <ReplaceButton
        content="x"
        onReplace={vi.fn().mockResolvedValue(false)}
      />,
    );
    expect(screen.getByRole('button', { name: LABEL })).toBeInTheDocument();
  });

  it('calls onReplace with the content on click', async () => {
    const onReplace = vi.fn().mockResolvedValue(false);
    render(<ReplaceButton content="rewritten text" onReplace={onReplace} />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: LABEL }));
    });
    expect(onReplace).toHaveBeenCalledWith('rewritten text');
  });

  it('shows a hover tooltip (same Tooltip used by the chat header icons)', () => {
    render(
      <ReplaceButton
        content="x"
        onReplace={vi.fn().mockResolvedValue(false)}
      />,
    );
    fireEvent.mouseEnter(
      screen.getByRole('button', { name: LABEL }).parentElement!,
    );
    expect(screen.getByText('Replace selection')).toBeInTheDocument();
  });

  it('shows a checkmark after a successful replace (aria-label becomes "Replaced")', async () => {
    render(
      <ReplaceButton content="x" onReplace={vi.fn().mockResolvedValue(true)} />,
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: LABEL }));
    });
    expect(
      screen.getByRole('button', { name: 'Replaced' }),
    ).toBeInTheDocument();
  });

  it('stays in the default state when the replace is skipped', async () => {
    render(
      <ReplaceButton
        content="x"
        onReplace={vi.fn().mockResolvedValue(false)}
      />,
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: LABEL }));
    });
    expect(screen.getByRole('button', { name: LABEL })).toBeInTheDocument();
  });

  it('reverts to the replace icon after 1.5 seconds', async () => {
    vi.useFakeTimers();
    render(
      <ReplaceButton content="x" onReplace={vi.fn().mockResolvedValue(true)} />,
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: LABEL }));
    });
    expect(
      screen.getByRole('button', { name: 'Replaced' }),
    ).toBeInTheDocument();
    act(() => {
      vi.advanceTimersByTime(1500);
    });
    expect(screen.getByRole('button', { name: LABEL })).toBeInTheDocument();
    vi.useRealTimers();
  });

  it('clears the prior revert timer on a rapid second replace', async () => {
    const onReplace = vi.fn().mockResolvedValue(true);
    render(<ReplaceButton content="x" onReplace={onReplace} />);
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: LABEL }));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Replaced' }));
    });
    expect(onReplace).toHaveBeenCalledTimes(2);
  });
});
