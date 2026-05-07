import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { UpdateFooterBar } from '../UpdateFooterBar';

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
});

describe('UpdateFooterBar', () => {
  const baseProps = {
    version: '0.8.0',
    notesUrl: 'https://github.com/quiet-node/thuki/releases/tag/v0.8.0',
    onInstall: vi.fn(),
    onLater: vi.fn(),
  };

  it('renders UPD pill and version', () => {
    render(<UpdateFooterBar {...baseProps} />);
    expect(screen.getByText('UPD')).toBeInTheDocument();
    expect(screen.getByText('v0.8.0')).toBeInTheDocument();
  });

  it('calls onInstall when install link clicked', () => {
    const onInstall = vi.fn();
    render(<UpdateFooterBar {...baseProps} onInstall={onInstall} />);
    fireEvent.click(screen.getByText(/install & restart/i));
    expect(onInstall).toHaveBeenCalled();
  });

  it('calls onLater when later link clicked', () => {
    const onLater = vi.fn();
    render(<UpdateFooterBar {...baseProps} onLater={onLater} />);
    fireEvent.click(screen.getByText('later'));
    expect(onLater).toHaveBeenCalled();
  });

  it('opens notesUrl via open_url when version button clicked', () => {
    render(<UpdateFooterBar {...baseProps} />);
    fireEvent.click(screen.getByText('v0.8.0'));
    expect(invokeMock).toHaveBeenCalledWith('open_url', {
      url: 'https://github.com/quiet-node/thuki/releases/tag/v0.8.0',
    });
  });

  it('does not invoke open_url when notesUrl is null and version clicked', () => {
    render(<UpdateFooterBar {...baseProps} notesUrl={null} />);
    fireEvent.click(screen.getByText('v0.8.0'));
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('renders with data-testid="update-footer-bar"', () => {
    render(<UpdateFooterBar {...baseProps} />);
    expect(screen.getByTestId('update-footer-bar')).toBeInTheDocument();
  });
});
