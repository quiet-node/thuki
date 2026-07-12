import { render } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { TypingIndicator } from '../TypingIndicator';

describe('TypingIndicator', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn().mockReturnValue({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      }),
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('renders the unified three-dot motion host', () => {
    const { getByTestId } = render(<TypingIndicator />);
    expect(getByTestId('three-dot-motion')).toBeInTheDocument();
  });

  it('renders without AI bubble wrapper (no chat-bubble-ai class)', () => {
    const { container } = render(<TypingIndicator />);
    expect(container.querySelector('.chat-bubble-ai')).toBeNull();
  });

  it('is left-aligned (justify-start on outer wrapper)', () => {
    const { container } = render(<TypingIndicator />);
    expect(
      container.firstElementChild?.classList.contains('justify-start'),
    ).toBe(true);
  });

  it('has accessible status role and label on the motion host', () => {
    const { getByRole } = render(<TypingIndicator />);
    const status = getByRole('status');
    expect(status).toHaveAttribute('aria-label', 'AI is thinking');
  });

  it('renders three leader dots (middle brand)', () => {
    const { container, getByTestId } = render(<TypingIndicator />);
    expect(container.querySelectorAll('.tdm-dot')).toHaveLength(3);
    expect(getByTestId('tdm-dot-brand')).toBeInTheDocument();
  });
});
