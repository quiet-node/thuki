import { render } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { TypingIndicator } from '../TypingIndicator';

describe('TypingIndicator', () => {
  it('renders exactly three dots (elements with rounded-full and bg-primary/70)', () => {
    const { container } = render(<TypingIndicator />);
    const dots = container.querySelectorAll('.rounded-full.bg-primary\\/70');
    expect(dots).toHaveLength(3);
  });

  it('renders with AI bubble styling (chat-bubble-ai)', () => {
    const { container } = render(<TypingIndicator />);
    expect(container.querySelector('.chat-bubble-ai')).toBeTruthy();
  });

  it('is left-aligned (justify-start)', () => {
    const { container } = render(<TypingIndicator />);
    const outerDiv = container.firstElementChild;
    expect(outerDiv?.classList.contains('justify-start')).toBe(true);
  });

  it('dots have correct size classes (w-2 h-2)', () => {
    const { container } = render(<TypingIndicator />);
    const dots = container.querySelectorAll('.w-2.h-2');
    expect(dots.length).toBeGreaterThanOrEqual(3);
  });
});
