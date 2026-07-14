import { describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import {
  PointingLabel,
  PointingWiggle,
  POINTING_WIGGLE_MS,
} from '../PointingWiggle';

describe('PointingWiggle', () => {
  it('exports the full animation duration', () => {
    expect(POINTING_WIGGLE_MS).toBe(7200);
  });

  it('renders nothing when inactive', () => {
    const { container } = render(<PointingWiggle active={false} />);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders the squiggle when active', () => {
    render(<PointingWiggle active testId="wiggle-a" />);
    expect(screen.getByTestId('wiggle-a')).toBeInTheDocument();
  });
});

describe('PointingLabel', () => {
  it('always shows children and only wiggles when active', () => {
    const { rerender } = render(
      <PointingLabel active={false}>Discover</PointingLabel>,
    );
    expect(screen.getByText('Discover')).toBeInTheDocument();
    expect(screen.queryByTestId('pointing-wiggle')).toBeNull();
    rerender(<PointingLabel active>Discover</PointingLabel>);
    expect(screen.getByTestId('pointing-wiggle')).toBeInTheDocument();
  });

  it('appends an optional className on the label wrap', () => {
    const { container } = render(
      <PointingLabel className="extra-class">Label</PointingLabel>,
    );
    expect(container.firstElementChild?.className).toContain('extra-class');
  });
});
