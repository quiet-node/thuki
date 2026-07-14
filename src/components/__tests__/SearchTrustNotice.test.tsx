import { describe, expect, it, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import {
  SearchTrustNotice,
  SEARCH_TRUST_NOTICE_BODY,
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

  it('renders approved title and body copy', () => {
    render(
      <SearchTrustNotice onAcknowledge={() => {}} onOpenSettings={() => {}} />,
    );
    expect(screen.getByText(SEARCH_TRUST_NOTICE_TITLE)).toBeTruthy();
    expect(screen.getByText(SEARCH_TRUST_NOTICE_BODY)).toBeTruthy();
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

  it('hides How search works until a blog disclosure URL is configured', () => {
    render(
      <SearchTrustNotice onAcknowledge={() => {}} onOpenSettings={() => {}} />,
    );
    expect(screen.queryByTestId('search-trust-notice-how')).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });
});
