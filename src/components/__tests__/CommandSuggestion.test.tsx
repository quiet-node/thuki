import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { CommandSuggestion } from '../CommandSuggestion';
import type { Command } from '../../config/commands';

const SCREEN_CMD: Command = {
  trigger: '/screen',
  label: '/screen',
  description: 'Capture your screen and include it as context',
};

const FOO_CMD: Command = {
  trigger: '/foo',
  label: '/foo',
  description: 'A test command',
};

describe('CommandSuggestion', () => {
  it('shows "No commands found" when commands list is empty', () => {
    render(
      <CommandSuggestion
        commands={[]}
        highlightedIndex={0}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByText('No commands found')).toBeInTheDocument();
  });

  it('renders each command trigger and description', () => {
    render(
      <CommandSuggestion
        commands={[SCREEN_CMD, FOO_CMD]}
        highlightedIndex={0}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByText('/screen')).toBeInTheDocument();
    expect(
      screen.getByText('Capture your screen and include it as context'),
    ).toBeInTheDocument();
    expect(screen.getByText('/foo')).toBeInTheDocument();
    expect(screen.getByText('A test command')).toBeInTheDocument();
  });

  it('shows the COMMANDS header', () => {
    render(
      <CommandSuggestion
        commands={[SCREEN_CMD]}
        highlightedIndex={0}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.getByText('Commands')).toBeInTheDocument();
  });

  it('marks the highlighted row as aria-selected', () => {
    render(
      <CommandSuggestion
        commands={[SCREEN_CMD, FOO_CMD]}
        highlightedIndex={1}
        onSelect={vi.fn()}
      />,
    );
    const options = screen.getAllByRole('option');
    expect(options[0]).toHaveAttribute('aria-selected', 'false');
    expect(options[1]).toHaveAttribute('aria-selected', 'true');
  });

  it('shows Tab badge only on highlighted row', () => {
    render(
      <CommandSuggestion
        commands={[SCREEN_CMD, FOO_CMD]}
        highlightedIndex={0}
        onSelect={vi.fn()}
      />,
    );
    // Only one Tab badge should appear.
    const tabBadges = screen.getAllByText('Tab');
    expect(tabBadges).toHaveLength(1);
  });

  it('shows no Tab badge when nothing is highlighted (index -1)', () => {
    render(
      <CommandSuggestion
        commands={[SCREEN_CMD]}
        highlightedIndex={-1}
        onSelect={vi.fn()}
      />,
    );
    expect(screen.queryByText('Tab')).toBeNull();
  });

  it('calls onSelect with the trigger when a row is clicked (mousedown)', () => {
    const onSelect = vi.fn();
    render(
      <CommandSuggestion
        commands={[SCREEN_CMD]}
        highlightedIndex={0}
        onSelect={onSelect}
      />,
    );
    const option = screen.getByRole('option');
    fireEvent.mouseDown(option);
    expect(onSelect).toHaveBeenCalledWith('/screen');
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  it('calls onSelect with the correct trigger when second row is clicked', () => {
    const onSelect = vi.fn();
    render(
      <CommandSuggestion
        commands={[SCREEN_CMD, FOO_CMD]}
        highlightedIndex={0}
        onSelect={onSelect}
      />,
    );
    const options = screen.getAllByRole('option');
    fireEvent.mouseDown(options[1]);
    expect(onSelect).toHaveBeenCalledWith('/foo');
  });

  it('renders the listbox with accessible label', () => {
    render(
      <CommandSuggestion
        commands={[SCREEN_CMD]}
        highlightedIndex={0}
        onSelect={vi.fn()}
      />,
    );
    expect(
      screen.getByRole('listbox', { name: 'Command suggestions' }),
    ).toBeInTheDocument();
  });

  it('does not throw when highlightedIndex is out of range', () => {
    expect(() => {
      render(
        <CommandSuggestion
          commands={[SCREEN_CMD]}
          highlightedIndex={99}
          onSelect={vi.fn()}
        />,
      );
    }).not.toThrow();
  });
});
