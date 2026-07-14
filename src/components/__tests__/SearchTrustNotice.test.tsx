import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import {
  SearchTrustNotice,
  SEARCH_DISCLOSURE_URL,
  SEARCH_TRUST_NOTICE_BODY,
  SEARCH_TRUST_NOTICE_LEARN_LABEL,
  SEARCH_TRUST_NOTICE_TITLE,
} from '../SearchTrustNotice';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));

describe('SearchTrustNotice', () => {
  beforeEach(() => {
    invoke.mockReset();
  });

  it('renders approved title, one-paragraph body, and in-body learn link', () => {
    render(
      <SearchTrustNotice onAcknowledge={() => {}} onOpenSettings={() => {}} />,
    );
    expect(screen.getByText(SEARCH_TRUST_NOTICE_TITLE)).toBeTruthy();
    expect(screen.getByText(SEARCH_TRUST_NOTICE_BODY, { exact: false })).toBeTruthy();
    expect(screen.getByText(SEARCH_TRUST_NOTICE_LEARN_LABEL)).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'How Auto search works' })).toBeNull();
  });

  it('uses elevated warm panel chrome without a version kicker', () => {
    render(
      <SearchTrustNotice onAcknowledge={() => {}} onOpenSettings={() => {}} />,
    );
    const root = screen.getByTestId('search-trust-notice');
    expect(root.className).toContain('bg-surface-elevated');
    expect(root.className).toContain('border-surface-border');
    expect(root.textContent).not.toMatch(/^v0\.16\.0/);
    expect(screen.queryByText('v0.16.0', { exact: true })).toBeNull();
  });

  it('Got it calls onAcknowledge', () => {
    const onAcknowledge = vi.fn();
    render(
      <SearchTrustNotice
        onAcknowledge={onAcknowledge}
        onOpenSettings={() => {}}
      />,
    );
    fireEvent.click(screen.getByTestId('search-trust-notice-got-it'));
    expect(onAcknowledge).toHaveBeenCalledTimes(1);
  });

  it('Turn off in Settings calls onOpenSettings without toggling search', () => {
    const onOpenSettings = vi.fn();
    render(
      <SearchTrustNotice
        onAcknowledge={() => {}}
        onOpenSettings={onOpenSettings}
      />,
    );
    fireEvent.click(screen.getByTestId('search-trust-notice-settings'));
    expect(onOpenSettings).toHaveBeenCalledTimes(1);
    expect(invoke).not.toHaveBeenCalledWith(
      'set_config_field',
      expect.anything(),
    );
  });

  it('See how Auto search works opens blog URL via open_url', () => {
    render(
      <SearchTrustNotice onAcknowledge={() => {}} onOpenSettings={() => {}} />,
    );
    fireEvent.click(screen.getByTestId('search-trust-notice-how'));
    expect(invoke).toHaveBeenCalledWith('open_url', {
      url: SEARCH_DISCLOSURE_URL,
    });
    expect(SEARCH_DISCLOSURE_URL).toBe('https://thuki.app/blog');
  });
});
