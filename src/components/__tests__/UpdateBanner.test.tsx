import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { UpdateBanner } from '../UpdateBanner';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe('UpdateBanner', () => {
  const baseProps = {
    version: '0.8.0',
    notesUrl: 'https://github.com/quiet-node/thuki/releases/tag/v0.8.0',
    onInstall: vi.fn(),
    onLater: vi.fn(),
  };

  it('renders title with version', () => {
    render(<UpdateBanner {...baseProps} />);
    expect(screen.getByText(/0\.8\.0/)).toBeInTheDocument();
  });

  it('calls onInstall when Install clicked', () => {
    const onInstall = vi.fn();
    render(<UpdateBanner {...baseProps} onInstall={onInstall} />);
    fireEvent.click(screen.getByRole('button', { name: /install & restart/i }));
    expect(onInstall).toHaveBeenCalled();
  });

  it('calls onLater when Later clicked', () => {
    const onLater = vi.fn();
    render(<UpdateBanner {...baseProps} onLater={onLater} />);
    fireEvent.click(screen.getByRole('button', { name: /later/i }));
    expect(onLater).toHaveBeenCalled();
  });

  it('renders Release notes button when notesUrl is provided and opens it via open_url', () => {
    render(<UpdateBanner {...baseProps} />);
    const notesBtn = screen.getByRole('button', { name: /release notes/i });
    expect(notesBtn).toBeInTheDocument();
    fireEvent.click(notesBtn);
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/releases/tag/v0.8.0',
    });
  });

  it('renders Update ready fallback when notesUrl is null', () => {
    render(<UpdateBanner {...baseProps} notesUrl={null} />);
    expect(screen.getByText('Update ready')).toBeInTheDocument();
  });

  it('renders with role=status and aria-live=polite', () => {
    render(<UpdateBanner {...baseProps} />);
    const banner = screen.getByRole('status');
    expect(banner).toHaveAttribute('aria-live', 'polite');
  });
});
