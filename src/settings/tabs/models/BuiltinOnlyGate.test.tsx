/**
 * Unit tests for the built-in-only gate: the overlay shown over Library and
 * Discover while a non-built-in provider is active. The gate keeps the real
 * pane mounted behind glass (so the user sees what is waiting) and offers a
 * one-click switch back to the built-in engine.
 */

import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { BuiltinOnlyGate } from './BuiltinOnlyGate';

describe('BuiltinOnlyGate', () => {
  it('renders children unchanged when not gated', () => {
    render(
      <BuiltinOnlyGate gated={false} activeLabel="Ollama" onSwitch={() => {}}>
        <p>pane content</p>
      </BuiltinOnlyGate>,
    );
    expect(screen.getByText('pane content')).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Switch to built-in' }),
    ).toBeNull();
  });

  it('overlays a switch prompt naming the active provider when gated', () => {
    render(
      <BuiltinOnlyGate gated activeLabel="Ollama" onSwitch={() => {}}>
        <p>pane content</p>
      </BuiltinOnlyGate>,
    );
    expect(
      screen.getByRole('button', { name: 'Switch to built-in' }),
    ).toBeInTheDocument();
    expect(screen.getByText(/You're using Ollama now/)).toBeInTheDocument();
  });

  it('keeps the gated children mounted but hidden from assistive tech', () => {
    render(
      <BuiltinOnlyGate gated activeLabel="Ollama" onSwitch={() => {}}>
        <p>pane content</p>
      </BuiltinOnlyGate>,
    );
    const child = screen.getByText('pane content');
    expect(child).toBeInTheDocument();
    expect(child.closest('[aria-hidden="true"]')).not.toBeNull();
  });

  it('marks the gated children inert so keyboard focus cannot reach them', () => {
    render(
      <BuiltinOnlyGate gated activeLabel="Ollama" onSwitch={() => {}}>
        <button>hidden action</button>
      </BuiltinOnlyGate>,
    );
    expect(screen.getByText('hidden action').closest('[inert]')).not.toBeNull();
  });

  it('calls onSwitch when the switch button is clicked', () => {
    const onSwitch = vi.fn();
    render(
      <BuiltinOnlyGate gated activeLabel="Ollama" onSwitch={onSwitch}>
        <p>pane content</p>
      </BuiltinOnlyGate>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'Switch to built-in' }));
    expect(onSwitch).toHaveBeenCalledTimes(1);
  });
});
