import { fireEvent, render, screen } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { CapabilityMismatchStrip } from '../CapabilityMismatchStrip';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

describe('CapabilityMismatchStrip', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('renders the message verbatim', () => {
    render(<CapabilityMismatchStrip message="llama3 can't see images." />);
    const strip = screen.getByTestId('capability-mismatch-strip');
    expect(strip).toHaveTextContent("llama3 can't see images.");
  });

  it('exposes role=status for assistive tech', () => {
    render(<CapabilityMismatchStrip message="x" />);
    expect(screen.getByTestId('capability-mismatch-strip')).toHaveAttribute(
      'role',
      'status',
    );
  });

  it('renders the inline-link variant with before/link/after segments', () => {
    render(
      <CapabilityMismatchStrip
        message={{
          before: 'Use an ',
          link: {
            text: 'OCR-supported command',
            url: 'https://example.test/x',
          },
          after: ', or switch to a vision model.',
        }}
      />,
    );
    const strip = screen.getByTestId('capability-mismatch-strip');
    // Container stays a non-interactive div so only the link is clickable.
    expect(strip.tagName).toBe('DIV');
    expect(strip).toHaveTextContent(
      'Use an OCR-supported command ↗, or switch to a vision model.',
    );
    const link = screen.getByTestId('capability-mismatch-strip-link');
    expect(link.tagName).toBe('BUTTON');
    expect(link).toHaveTextContent('OCR-supported command ↗');
    expect(link).toHaveAttribute(
      'aria-label',
      'Open documentation: https://example.test/x',
    );
  });

  it('invokes open_url only when the inline link is clicked, not the surrounding strip', () => {
    render(
      <CapabilityMismatchStrip
        message={{
          before: 'Use an ',
          link: {
            text: 'OCR-supported command',
            url: 'https://example.test/x',
          },
          after: ', or switch to a vision model.',
        }}
      />,
    );
    // Clicking the surrounding strip does nothing.
    fireEvent.click(screen.getByTestId('capability-mismatch-strip'));
    expect(invoke).not.toHaveBeenCalled();

    // Clicking the inline link opens the URL.
    fireEvent.click(screen.getByTestId('capability-mismatch-strip-link'));
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://example.test/x',
    });
  });
});
