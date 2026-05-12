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

  it('renders linked variant as a button when message has a url', () => {
    render(
      <CapabilityMismatchStrip
        message={{ text: 'Use OCR commands', url: 'https://example.test/x' }}
      />,
    );
    const strip = screen.getByTestId('capability-mismatch-strip');
    expect(strip.tagName).toBe('BUTTON');
    expect(strip).toHaveTextContent('Use OCR commands');
    expect(strip).toHaveAttribute(
      'aria-label',
      'Open documentation: https://example.test/x',
    );
  });

  it('invokes open_url with the message url when the linked variant is clicked', () => {
    render(
      <CapabilityMismatchStrip
        message={{ text: 'Use OCR commands', url: 'https://example.test/x' }}
      />,
    );
    fireEvent.click(screen.getByTestId('capability-mismatch-strip'));
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://example.test/x',
    });
  });
});
