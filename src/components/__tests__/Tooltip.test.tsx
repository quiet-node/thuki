import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { Tooltip } from '../Tooltip';

describe('Tooltip', () => {
  it('renders children without showing tooltip initially', () => {
    render(
      <Tooltip label="Save conversation">
        <button type="button">Save</button>
      </Tooltip>,
    );
    expect(screen.getByRole('button', { name: 'Save' })).toBeInTheDocument();
    expect(screen.queryByText('Save conversation')).not.toBeInTheDocument();
  });

  it('shows tooltip label on mouse enter', () => {
    render(
      <Tooltip label="Save conversation">
        <button type="button">Save</button>
      </Tooltip>,
    );
    const wrapper = screen.getByRole('button', { name: 'Save' }).parentElement!;
    fireEvent.mouseEnter(wrapper);
    expect(screen.getByText('Save conversation')).toBeInTheDocument();
  });

  it('hides tooltip on mouse leave', () => {
    render(
      <Tooltip label="Save conversation">
        <button type="button">Save</button>
      </Tooltip>,
    );
    const wrapper = screen.getByRole('button', { name: 'Save' }).parentElement!;
    fireEvent.mouseEnter(wrapper);
    expect(screen.getByText('Save conversation')).toBeInTheDocument();
    fireEvent.mouseLeave(wrapper);
    expect(screen.queryByText('Save conversation')).not.toBeInTheDocument();
  });

  it('does not mount portal content before first hover (lazy activation)', () => {
    const { container } = render(
      <Tooltip label="New conversation">
        <button type="button">New</button>
      </Tooltip>,
    );
    // Before any hover, no portal content should exist in the document
    expect(
      document.body.querySelector('[style*="position: fixed"]'),
    ).toBeNull();
    // The wrapper div itself is in the container
    expect(container.querySelector('.inline-flex')).not.toBeNull();
  });

  it('renders portal content inside document.body on hover', () => {
    render(
      <Tooltip label="Conversation history">
        <button type="button">History</button>
      </Tooltip>,
    );
    const wrapper = screen.getByRole('button', {
      name: 'History',
    }).parentElement!;
    fireEvent.mouseEnter(wrapper);
    // Portal renders to document.body, outside the test container
    expect(document.body).toHaveTextContent('Conversation history');
  });

  it('remains visible on subsequent hovers after initial activation', () => {
    render(
      <Tooltip label="Open history">
        <button type="button">Btn</button>
      </Tooltip>,
    );
    const wrapper = screen.getByRole('button', { name: 'Btn' }).parentElement!;

    fireEvent.mouseEnter(wrapper);
    fireEvent.mouseLeave(wrapper);
    fireEvent.mouseEnter(wrapper);

    expect(screen.getByText('Open history')).toBeInTheDocument();
  });

  it('wraps children in an inline-flex div', () => {
    const { container } = render(
      <Tooltip label="Test">
        <button type="button">Trigger</button>
      </Tooltip>,
    );
    const wrapper = container.firstElementChild;
    expect(wrapper?.tagName.toLowerCase()).toBe('div');
    expect(wrapper?.classList.contains('inline-flex')).toBe(true);
  });
});
