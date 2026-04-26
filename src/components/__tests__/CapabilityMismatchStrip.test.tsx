import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { CapabilityMismatchStrip } from '../CapabilityMismatchStrip';

describe('CapabilityMismatchStrip', () => {
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
});
