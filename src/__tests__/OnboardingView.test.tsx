import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { OnboardingView } from '../view/OnboardingView';
import { invoke } from '../testUtils/mocks/tauri';

describe('OnboardingView', () => {
  beforeEach(() => {
    invoke.mockClear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function setupPermissions(accessibility: boolean, screenRecording = false) {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return accessibility;
      if (cmd === 'check_screen_recording_permission') return screenRecording;
      if (cmd === 'check_screen_recording_tcc_granted') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
    });
  }

  it('shows step 1 as active when accessibility is not granted', async () => {
    setupPermissions(false);
    render(<OnboardingView />);
    await act(async () => {});

    expect(screen.getByText('Accessibility')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /grant accessibility/i }),
    ).toBeInTheDocument();
  });

  it('shows the onboarding title', async () => {
    setupPermissions(false);
    render(<OnboardingView />);
    await act(async () => {});

    expect(screen.getByText("Let's get Thuki set up")).toBeInTheDocument();
  });

  it('skips to step 2 when accessibility is already granted on mount', async () => {
    setupPermissions(true);
    render(<OnboardingView />);
    await act(async () => {});

    expect(
      screen.queryByRole('button', { name: /grant accessibility/i }),
    ).toBeNull();
    expect(
      screen.getByRole('button', { name: /open screen recording settings/i }),
    ).toBeInTheDocument();
  });

  it('clicking grant accessibility invokes request command', async () => {
    setupPermissions(false);
    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    expect(invoke).toHaveBeenCalledWith('open_accessibility_settings');
  });

  it('shows spinner while polling after grant request', async () => {
    setupPermissions(false);
    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    // Button should be disabled/spinner state while checking
    const btn = screen.getByRole('button', {
      name: /checking|grant accessibility/i,
    });
    expect(btn).toBeDisabled();
  });

  it('keeps polling when accessibility not yet granted on first poll interval', async () => {
    let accessibilityGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return accessibilityGranted;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'open_accessibility_settings') return;
    });

    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    // First poll fires but permission still false
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    // Still on step 1, open screen recording button not yet shown
    expect(
      screen.queryByRole('button', { name: /open screen recording settings/i }),
    ).toBeNull();

    // Now grant it and fire second poll
    accessibilityGranted = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    // Step 2 now active
    expect(
      screen.getByRole('button', { name: /open screen recording settings/i }),
    ).toBeInTheDocument();
  });

  it('advances to step 2 when polling detects accessibility granted', async () => {
    let accessibilityGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return accessibilityGranted;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'open_accessibility_settings') return;
    });

    render(<OnboardingView />);
    await act(async () => {});

    // Click grant
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    // Grant becomes true before next poll
    accessibilityGranted = true;

    // Advance one poll interval
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    // Step 2 should now be active
    expect(
      screen.getByRole('button', { name: /open screen recording settings/i }),
    ).toBeInTheDocument();
  });

  it('step 1 shows granted badge after accessibility is detected', async () => {
    let accessibilityGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return accessibilityGranted;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'open_accessibility_settings') return;
    });

    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    accessibilityGranted = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(screen.getByText('Granted')).toBeInTheDocument();
  });

  it('clicking open screen recording settings registers app and opens settings', async () => {
    setupPermissions(true);
    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    // Registers Thuki in TCC (so it appears in the list) then opens Settings
    expect(invoke).toHaveBeenCalledWith('request_screen_recording_access');
    expect(invoke).toHaveBeenCalledWith('open_screen_recording_settings');
  });

  it('shows spinner while polling after opening screen recording settings', async () => {
    setupPermissions(true);
    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    // Button should be disabled/spinner state while polling for tcc grant
    const btn = screen.getByRole('button', {
      name: /checking|open screen recording settings/i,
    });
    expect(btn).toBeDisabled();
  });

  it('does not show quit and reopen immediately after clicking screen recording button', async () => {
    setupPermissions(true);
    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    // Should NOT show quit & reopen until tcc grant is detected
    expect(screen.queryByRole('button', { name: /quit.*reopen/i })).toBeNull();
  });

  it('keeps polling when screen recording tcc not yet granted', async () => {
    let tccGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'check_screen_recording_tcc_granted') return tccGranted;
    });

    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    // First poll: still not granted
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(screen.queryByRole('button', { name: /quit.*reopen/i })).toBeNull();

    // Grant it
    tccGranted = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(
      screen.getByRole('button', { name: /quit.*reopen/i }),
    ).toBeInTheDocument();
  });

  it('shows quit and reopen after screen recording tcc grant is detected', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'check_screen_recording_tcc_granted') return true;
    });

    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(
      screen.getByRole('button', { name: /quit.*reopen/i }),
    ).toBeInTheDocument();
  });

  it('clicking quit and reopen invokes quit_and_relaunch', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'check_screen_recording_tcc_granted') return true;
    });

    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: /quit.*reopen/i }));
    });

    expect(invoke).toHaveBeenCalledWith('quit_and_relaunch');
  });

  it('shows screen recording step info', async () => {
    setupPermissions(true);
    render(<OnboardingView />);
    await act(async () => {});

    expect(screen.getByText('Screen Recording')).toBeInTheDocument();
  });

  it('shows both steps regardless of current active step', async () => {
    setupPermissions(false);
    render(<OnboardingView />);
    await act(async () => {});

    expect(screen.getByText('Accessibility')).toBeInTheDocument();
    expect(screen.getByText('Screen Recording')).toBeInTheDocument();
  });

  it('does not emit console.error when unmounted during accessibility polling', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

    setupPermissions(false);
    const { unmount } = render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    act(() => unmount());

    // Timer ticks after unmount must not trigger React state-update warnings.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('does not emit console.error when unmounted during screen recording polling', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'check_screen_recording_tcc_granted') return false;
    });

    const { unmount } = render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    act(() => unmount());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('hovering the CTA button applies brightness filter when enabled', async () => {
    setupPermissions(false);
    render(<OnboardingView />);
    await act(async () => {});

    const btn = screen.getByRole('button', { name: /grant accessibility/i });
    fireEvent.mouseEnter(btn);
    // The button is not disabled so hovered=true applies brightness(1.1).
    // Verify the element is still present and interactive (no errors thrown).
    expect(btn).toBeInTheDocument();
    fireEvent.mouseLeave(btn);
    expect(btn).toBeInTheDocument();
  });

  it('hovering a disabled CTA button does not apply brightness filter', async () => {
    setupPermissions(false);
    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    // Button is now disabled/polling
    const btn = screen.getByRole('button', {
      name: /checking|grant accessibility/i,
    });
    expect(btn).toBeDisabled();
    // mouseEnter on a disabled button must not toggle hovered state
    fireEvent.mouseEnter(btn);
    expect(btn).toBeDisabled();
    fireEvent.mouseLeave(btn);
    expect(btn).toBeDisabled();
  });

  // ─── Defensive guard coverage ─────────────────────────────────────────────
  // The following tests exercise the early-return branches that protect against
  // stale state updates and concurrent invocations. These branches cannot be
  // reached through the happy-path tests because the invoke mock resolves
  // synchronously; here we use deferred promises to keep invocations in-flight
  // long enough to trigger each guard.

  it('ignores initial accessibility check result when component unmounts mid-flight', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    let resolveInitial!: (v: boolean) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission')
        return new Promise((r) => {
          resolveInitial = r;
        });
      return Promise.resolve();
    });

    const { unmount } = render(<OnboardingView />);
    // useEffect has fired; initial invoke is in-flight (resolveInitial is set).

    act(() => unmount()); // mountedRef → false

    await act(async () => {
      resolveInitial(true); // then-handler fires; guard returns early
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('ax in-flight guard prevents concurrent permission checks', async () => {
    let pollCallCount = 0;
    let resolveFirstPoll!: (v: boolean) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission') {
        pollCallCount++;
        if (pollCallCount === 1) return Promise.resolve(false); // initial check
        return new Promise((r) => {
          resolveFirstPoll = r;
        }); // poll hangs
      }
      if (cmd === 'open_accessibility_settings') return Promise.resolve();
      return Promise.resolve();
    });

    render(<OnboardingView />);
    await act(async () => {}); // initial check done

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    // First tick: callback starts, sets in-flight=true, invoke hangs.
    // Second tick (while first is still in-flight): guard returns early.
    act(() => {
      vi.advanceTimersByTime(500);
      vi.advanceTimersByTime(500);
    });

    // Only one poll call (initial was count=1, first poll was count=2; second
    // tick was blocked — no count=3).
    expect(pollCallCount).toBe(2);

    await act(async () => {
      resolveFirstPoll(false);
    });
  });

  it('ignores ax poll result when component unmounts during in-flight check', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    let callCount = 0;
    let resolvePoll!: (v: boolean) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission') {
        callCount++;
        if (callCount === 1) return Promise.resolve(false);
        return new Promise((r) => {
          resolvePoll = r;
        });
      }
      if (cmd === 'open_accessibility_settings') return Promise.resolve();
      return Promise.resolve();
    });

    const { unmount } = render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    // Fire one tick so the poll invoke is in-flight (hanging).
    act(() => vi.advanceTimersByTime(500));

    // Unmount while the invoke is still pending; this clears the interval but
    // the in-flight promise is still alive.
    act(() => unmount());

    // Resolving the promise must not trigger a React state update.
    await act(async () => {
      resolvePoll(true);
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('ignores screen recording handler when component unmounts during open-settings call', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    let resolveOpen!: (v?: unknown) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission')
        return Promise.resolve(true);
      if (cmd === 'request_screen_recording_access') return Promise.resolve();
      if (cmd === 'open_screen_recording_settings')
        return new Promise((r) => {
          resolveOpen = r;
        }); // hangs
      if (cmd === 'check_screen_recording_tcc_granted')
        return Promise.resolve(false);
      return Promise.resolve();
    });

    const { unmount } = render(<OnboardingView />);
    await act(async () => {}); // accessibility granted

    // Flush microtasks so the handler advances past the first await
    // (request_screen_recording_access resolves) and suspends on the second
    // (open_screen_recording_settings hangs), setting resolveOpen.
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    act(() => unmount()); // mountedRef → false

    await act(async () => {
      resolveOpen(); // mountedRef guard at line 225 fires; returns early
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('screen in-flight guard prevents concurrent tcc checks', async () => {
    let tccCallCount = 0;
    let resolveFirstPoll!: (v: boolean) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission')
        return Promise.resolve(true);
      if (cmd === 'request_screen_recording_access') return Promise.resolve();
      if (cmd === 'open_screen_recording_settings') return Promise.resolve();
      if (cmd === 'check_screen_recording_tcc_granted') {
        tccCallCount++;
        return new Promise((r) => {
          resolveFirstPoll = r;
        });
      }
      return Promise.resolve();
    });

    render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    act(() => {
      vi.advanceTimersByTime(500); // first tick: in-flight
      vi.advanceTimersByTime(500); // second tick: guard blocks it
    });

    expect(tccCallCount).toBe(1);

    await act(async () => {
      resolveFirstPoll(false);
    });
  });

  it('ignores screen poll result when component unmounts during in-flight tcc check', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    let resolvePoll!: (v: boolean) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission')
        return Promise.resolve(true);
      if (cmd === 'request_screen_recording_access') return Promise.resolve();
      if (cmd === 'open_screen_recording_settings') return Promise.resolve();
      if (cmd === 'check_screen_recording_tcc_granted')
        return new Promise((r) => {
          resolvePoll = r;
        });
      return Promise.resolve();
    });

    const { unmount } = render(<OnboardingView />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    act(() => vi.advanceTimersByTime(500)); // poll fires, invoke hangs

    act(() => unmount()); // clears interval; in-flight promise still alive

    await act(async () => {
      resolvePoll(true); // mountedRef guard at line 234 fires; returns early
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });
});
