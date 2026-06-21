import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { ModelsSegmented } from './ModelsSegmented';

describe('ModelsSegmented', () => {
  it('renders the three model views', () => {
    render(<ModelsSegmented value="providers" onChange={() => {}} />);
    expect(screen.getByRole('tab', { name: 'Library' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Discover' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Providers' })).toBeInTheDocument();
  });

  it('renders a decorative icon in each tab', () => {
    const { container } = render(
      <ModelsSegmented value="library" onChange={() => {}} />,
    );
    expect(container.querySelectorAll('svg')).toHaveLength(3);
  });

  it('marks the active view as selected', () => {
    render(<ModelsSegmented value="discover" onChange={() => {}} />);
    expect(screen.getByRole('tab', { name: 'Discover' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    expect(screen.getByRole('tab', { name: 'Library' })).toHaveAttribute(
      'aria-selected',
      'false',
    );
  });

  it('calls onChange with the clicked view', () => {
    const onChange = vi.fn();
    render(<ModelsSegmented value="providers" onChange={onChange} />);
    fireEvent.click(screen.getByRole('tab', { name: 'Library' }));
    expect(onChange).toHaveBeenCalledWith('library');
  });

  it('ArrowRight selects the next view', () => {
    const onChange = vi.fn();
    render(<ModelsSegmented value="library" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Library' }), {
      key: 'ArrowRight',
    });
    expect(onChange).toHaveBeenCalledWith('discover');
  });

  it('ArrowRight wraps from the last view to the first', () => {
    const onChange = vi.fn();
    render(<ModelsSegmented value="providers" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Providers' }), {
      key: 'ArrowRight',
    });
    expect(onChange).toHaveBeenCalledWith('library');
  });

  it('ArrowLeft wraps from the first view to the last', () => {
    const onChange = vi.fn();
    render(<ModelsSegmented value="library" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Library' }), {
      key: 'ArrowLeft',
    });
    expect(onChange).toHaveBeenCalledWith('providers');
  });

  it('ignores non-arrow keys', () => {
    const onChange = vi.fn();
    render(<ModelsSegmented value="library" onChange={onChange} />);
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Library' }), {
      key: 'Enter',
    });
    expect(onChange).not.toHaveBeenCalled();
  });
});
