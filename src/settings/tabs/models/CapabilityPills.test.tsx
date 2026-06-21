/**
 * Unit tests for {@link CapabilityPills}, the shared Text / Vision / Thinking
 * capability badge row used by both the Staff-picks and Browse-all Discover
 * panes. Text is unconditional; Vision and Thinking are flag-gated.
 */

import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { CapabilityPills } from './CapabilityPills';

describe('CapabilityPills', () => {
  it('always renders the Text pill', () => {
    render(<CapabilityPills vision={false} thinking={false} />);
    expect(screen.getByText('Text')).toBeInTheDocument();
    expect(screen.queryByText('Vision')).not.toBeInTheDocument();
    expect(screen.queryByText('Thinking')).not.toBeInTheDocument();
  });

  it('renders the Vision pill only when vision is set', () => {
    render(<CapabilityPills vision thinking={false} />);
    expect(screen.getByText('Vision')).toBeInTheDocument();
    expect(screen.queryByText('Thinking')).not.toBeInTheDocument();
  });

  it('renders the Thinking pill only when thinking is set', () => {
    render(<CapabilityPills vision={false} thinking />);
    expect(screen.getByText('Thinking')).toBeInTheDocument();
    expect(screen.queryByText('Vision')).not.toBeInTheDocument();
  });

  it('renders Vision and Thinking together when both are set', () => {
    render(<CapabilityPills vision thinking />);
    expect(screen.getByText('Text')).toBeInTheDocument();
    expect(screen.getByText('Vision')).toBeInTheDocument();
    expect(screen.getByText('Thinking')).toBeInTheDocument();
  });
});
