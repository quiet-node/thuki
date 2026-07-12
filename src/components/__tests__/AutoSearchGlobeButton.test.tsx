/**
 * Tests for the ask-bar Auto search globe control.
 */

import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import { AutoSearchGlobeButton } from '../AutoSearchGlobeButton';
import {
  ConfigProviderForTest,
  DEFAULT_CONFIG,
  type AppConfig,
} from '../../contexts/ConfigContext';

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}));

const invokeMock = vi.mocked(invoke);

/**
 * Renders the globe under a fixed config snapshot.
 *
 * @param behaviorAutoSearch Value for `behavior.autoSearch`.
 * @param disabled Optional disabled prop.
 */
function renderGlobe(behaviorAutoSearch: boolean, disabled = false) {
  const value: AppConfig = {
    ...DEFAULT_CONFIG,
    behavior: {
      ...DEFAULT_CONFIG.behavior,
      autoSearch: behaviorAutoSearch,
    },
  };
  return render(
    <ConfigProviderForTest value={value}>
      <AutoSearchGlobeButton disabled={disabled} />
    </ConfigProviderForTest>,
  );
}

describe('AutoSearchGlobeButton', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue({});
  });

  it('shows the on-state aria label when auto search is enabled', () => {
    renderGlobe(true);
    const btn = screen.getByTestId('auto-search-globe');
    expect(btn).toHaveAttribute('aria-pressed', 'true');
    expect(btn).toHaveAttribute(
      'aria-label',
      expect.stringContaining('Auto search on'),
    );
  });

  it('shows the on-demand aria label when auto search is off', () => {
    renderGlobe(false);
    const btn = screen.getByTestId('auto-search-globe');
    expect(btn).toHaveAttribute('aria-pressed', 'false');
    expect(btn).toHaveAttribute(
      'aria-label',
      expect.stringContaining('On demand'),
    );
  });

  it('writes behavior.auto_search false when toggled from on', async () => {
    renderGlobe(true);
    fireEvent.click(screen.getByTestId('auto-search-globe'));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('set_config_field', {
        section: 'behavior',
        key: 'auto_search',
        value: false,
      });
    });
    expect(screen.getByTestId('auto-search-globe')).toHaveAttribute(
      'aria-pressed',
      'false',
    );
  });

  it('writes behavior.auto_search true when toggled from off', async () => {
    renderGlobe(false);
    fireEvent.click(screen.getByTestId('auto-search-globe'));
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith('set_config_field', {
        section: 'behavior',
        key: 'auto_search',
        value: true,
      });
    });
  });

  it('rolls back optimistic state when set_config_field fails', async () => {
    invokeMock.mockRejectedValueOnce(new Error('disk full'));
    renderGlobe(true);
    fireEvent.click(screen.getByTestId('auto-search-globe'));
    await waitFor(() => {
      expect(screen.getByTestId('auto-search-globe')).toHaveAttribute(
        'aria-pressed',
        'true',
      );
    });
  });

  it('does not invoke when disabled', () => {
    renderGlobe(true, true);
    fireEvent.click(screen.getByTestId('auto-search-globe'));
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it('disables the control while a write is in flight', async () => {
    let resolveWrite: (value: unknown) => void = () => {};
    invokeMock.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveWrite = resolve;
        }),
    );
    renderGlobe(true);
    const btn = screen.getByTestId('auto-search-globe');
    fireEvent.click(btn);
    await waitFor(() => {
      expect(btn).toBeDisabled();
    });
    resolveWrite({});
    await waitFor(() => {
      expect(btn).not.toBeDisabled();
      expect(btn).toHaveAttribute('aria-pressed', 'false');
    });
  });
});
