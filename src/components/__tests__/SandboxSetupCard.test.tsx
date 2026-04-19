import { render, screen } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { SandboxSetupCard } from '../SandboxSetupCard';

describe('SandboxSetupCard', () => {
  it('renders the setup card with testid', () => {
    render(<SandboxSetupCard />);
    expect(screen.getByTestId('sandbox-setup-card')).toBeInTheDocument();
  });

  it('shows "Search sandbox not running" as the title', () => {
    render(<SandboxSetupCard />);
    expect(screen.getByText('Search sandbox not running')).toBeInTheDocument();
  });

  it('shows the start command in a code element', () => {
    render(<SandboxSetupCard />);
    expect(screen.getByText('bun run search-box:start')).toBeInTheDocument();
  });

  it('renders the amber warning bar', () => {
    const { container } = render(<SandboxSetupCard />);
    // The warning bar carries a data-warning-bar attribute for test targeting.
    const bar = container.querySelector('[data-warning-bar]');
    expect(bar).not.toBeNull();
  });
});
