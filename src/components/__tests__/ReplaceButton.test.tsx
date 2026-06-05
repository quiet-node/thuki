import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { ReplaceButton } from '../ReplaceButton';

const LABEL = 'Replace selection in source app';

describe('ReplaceButton', () => {
  it('renders an accessible button', () => {
    render(<ReplaceButton content="x" onReplace={vi.fn()} />);
    expect(screen.getByRole('button', { name: LABEL })).toBeInTheDocument();
  });

  it('calls onReplace with the content on click', () => {
    const onReplace = vi.fn();
    render(<ReplaceButton content="rewritten text" onReplace={onReplace} />);
    fireEvent.click(screen.getByRole('button', { name: LABEL }));
    expect(onReplace).toHaveBeenCalledWith('rewritten text');
  });

  it('shows a hover tooltip (same Tooltip used by the chat header icons)', () => {
    render(<ReplaceButton content="x" onReplace={vi.fn()} />);
    fireEvent.mouseEnter(
      screen.getByRole('button', { name: LABEL }).parentElement!,
    );
    expect(screen.getByText('Replace selection')).toBeInTheDocument();
  });
});
