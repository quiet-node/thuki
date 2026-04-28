import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { SandboxSetupCard } from '../SandboxSetupCard';
import { invoke } from '@tauri-apps/api/core';

// Mock Tauri invoke
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

describe('SandboxSetupCard', () => {
  it('renders the setup card with testid', () => {
    render(<SandboxSetupCard />);
    expect(screen.getByTestId('sandbox-setup-card')).toBeInTheDocument();
  });

  it('shows "Search service is offline" as the title', () => {
    render(<SandboxSetupCard />);
    expect(screen.getByText('Search service is offline')).toBeInTheDocument();
  });

  it('shows the setup message and button', () => {
    render(<SandboxSetupCard />);
    expect(screen.getByText(/Follow the/i)).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /Setup Guide/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/to enable local agentic search./i),
    ).toBeInTheDocument();
  });

  it('invokes open_url when the button is clicked', () => {
    render(<SandboxSetupCard />);
    const button = screen.getByRole('button', { name: /Setup Guide/i });
    fireEvent.click(button);
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki#step-2-setup-the-search-sandbox-optional-required-for-search',
    });
  });

  it('renders the amber warning bar', () => {
    const { container } = render(<SandboxSetupCard />);
    // The warning bar carries a data-warning-bar attribute for test targeting.
    const bar = container.querySelector('[data-warning-bar]');
    expect(bar).not.toBeNull();
  });
});
