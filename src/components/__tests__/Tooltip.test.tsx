import { render, screen, fireEvent, act } from '@testing-library/react';
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

  it('hides tooltip when window regains focus (e.g. after screenshot)', () => {
    render(
      <Tooltip label="Take a screenshot">
        <button type="button">Capture</button>
      </Tooltip>,
    );
    const wrapper = screen.getByRole('button', {
      name: 'Capture',
    }).parentElement!;
    fireEvent.mouseEnter(wrapper);
    expect(screen.getByText('Take a screenshot')).toBeInTheDocument();

    act(() => {
      window.dispatchEvent(new Event('focus'));
    });

    expect(screen.queryByText('Take a screenshot')).not.toBeInTheDocument();
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

  it('hides on mouseDown so the tooltip does not overlap a popup the click opens', () => {
    render(
      <Tooltip label="Choose model">
        <button type="button">Trigger</button>
      </Tooltip>,
    );
    const wrapper = screen.getByRole('button', {
      name: 'Trigger',
    }).parentElement!;
    fireEvent.mouseEnter(wrapper);
    expect(screen.getByText('Choose model')).toBeInTheDocument();
    fireEvent.mouseDown(wrapper);
    expect(screen.queryByText('Choose model')).not.toBeInTheDocument();
  });

  it('renders the popup above the trigger when placement="top"', () => {
    render(
      <Tooltip label="Refresh now" multiline placement="top">
        <button type="button">Refresh</button>
      </Tooltip>,
    );
    const wrapper = screen.getByRole('button', {
      name: 'Refresh',
    }).parentElement!;
    fireEvent.mouseEnter(wrapper);
    expect(screen.getByText('Refresh now')).toBeInTheDocument();
  });

  it('applies extra className to the wrapper div', () => {
    const { container } = render(
      <Tooltip label="Test" className="flex-1 min-w-0">
        <span>content</span>
      </Tooltip>,
    );
    const wrapper = container.firstElementChild;
    expect(wrapper?.classList.contains('flex-1')).toBe(true);
    expect(wrapper?.classList.contains('min-w-0')).toBe(true);
    expect(wrapper?.classList.contains('inline-flex')).toBe(true);
  });

  it('renders multiline tooltips at a fixed 225px width so the box stays directly below the trigger near edges', () => {
    render(
      <Tooltip label={'Open by design: browse and pull any model.'} multiline>
        <button type="button">Trigger</button>
      </Tooltip>,
    );
    const wrapper = screen.getByRole('button', {
      name: 'Trigger',
    }).parentElement!;
    fireEvent.mouseEnter(wrapper);
    const fixedBox = document.body.querySelector(
      '[style*="position: fixed"]',
    ) as HTMLElement | null;
    expect(fixedBox).not.toBeNull();
    // The inner content div (under the fixed-positioned outer + motion
    // wrapper) carries the explicit 225px width style.
    const inner = fixedBox?.querySelector('div[style*="width"]');
    expect(inner).not.toBeNull();
    expect((inner as HTMLElement).style.width).toBe('225px');
  });
});
