import { renderHook, act, waitFor } from '@testing-library/react';
import { describe, it, expect, beforeEach } from 'vitest';
import { useUpdater } from '../useUpdater';
import { invoke, emitTauriEvent } from '../../testUtils/mocks/tauri';

const invokeMock = invoke as unknown as ReturnType<
  typeof import('vitest').vi.fn
>;

const SNAPSHOT_NO_UPDATE = {
  last_check_at_unix: null,
  update: null,
  settings_snoozed_until: null,
  chat_snoozed_until: null,
};

const SNAPSHOT_WITH_UPDATE = {
  last_check_at_unix: 100,
  update: { version: '0.8.0', notes_url: null },
  settings_snoozed_until: null,
  chat_snoozed_until: null,
};

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(SNAPSHOT_NO_UPDATE);
});

describe('useUpdater', () => {
  it('loads initial state from get_updater_state', async () => {
    invokeMock.mockResolvedValueOnce(SNAPSHOT_WITH_UPDATE);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() =>
      expect(result.current.state.update?.version).toBe('0.8.0'),
    );
    expect(invokeMock).toHaveBeenCalledWith('get_updater_state');
  });

  it('starts with EMPTY state before first invoke resolves', () => {
    invokeMock.mockImplementation(() => new Promise(() => {}));
    const { result } = renderHook(() => useUpdater());
    expect(result.current.state.update).toBeNull();
    expect(result.current.state.last_check_at_unix).toBeNull();
  });

  it('checkNow invokes check_for_update and refreshes state', async () => {
    invokeMock.mockResolvedValueOnce(SNAPSHOT_NO_UPDATE).mockResolvedValueOnce({
      last_check_at_unix: 200,
      update: { version: '0.9.0', notes_url: null },
      settings_snoozed_until: null,
      chat_snoozed_until: null,
    });
    const { result } = renderHook(() => useUpdater());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_updater_state'),
    );
    await act(async () => {
      await result.current.checkNow();
    });
    expect(invokeMock).toHaveBeenCalledWith('check_for_update');
    expect(result.current.state.update?.version).toBe('0.9.0');
  });

  it('snoozeChat invokes snooze_update_chat with hours and refreshes', async () => {
    invokeMock.mockResolvedValue(SNAPSHOT_NO_UPDATE);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_updater_state'),
    );
    await act(async () => {
      await result.current.snoozeChat(24);
    });
    expect(invokeMock).toHaveBeenCalledWith('snooze_update_chat', {
      hours: 24,
    });
    // Verify refresh fired after snooze (get_updater_state called again)
    expect(
      (invokeMock.mock.calls as unknown[][]).filter(
        (c) => c[0] === 'get_updater_state',
      ).length,
    ).toBeGreaterThanOrEqual(2);
  });

  it('snoozeSettings invokes snooze_update_settings with hours and refreshes', async () => {
    invokeMock.mockResolvedValue(SNAPSHOT_NO_UPDATE);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_updater_state'),
    );
    await act(async () => {
      await result.current.snoozeSettings(48);
    });
    expect(invokeMock).toHaveBeenCalledWith('snooze_update_settings', {
      hours: 48,
    });
    expect(
      (invokeMock.mock.calls as unknown[][]).filter(
        (c) => c[0] === 'get_updater_state',
      ).length,
    ).toBeGreaterThanOrEqual(2);
  });

  it('install invokes install_update', async () => {
    invokeMock.mockResolvedValue(SNAPSHOT_NO_UPDATE);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_updater_state'),
    );
    await act(async () => {
      await result.current.install();
    });
    expect(invokeMock).toHaveBeenCalledWith('install_update');
  });

  it('updates state when update-available event fires', async () => {
    invokeMock.mockResolvedValue(SNAPSHOT_NO_UPDATE);
    const { result } = renderHook(() => useUpdater());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_updater_state'),
    );
    act(() => {
      emitTauriEvent('update-available', SNAPSHOT_WITH_UPDATE);
    });
    await waitFor(() =>
      expect(result.current.state.update?.version).toBe('0.8.0'),
    );
  });

  it('unlistens from update-available on unmount without throwing', async () => {
    invokeMock.mockResolvedValue(SNAPSHOT_NO_UPDATE);
    const { unmount } = renderHook(() => useUpdater());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_updater_state'),
    );
    expect(() => unmount()).not.toThrow();
  });
});
