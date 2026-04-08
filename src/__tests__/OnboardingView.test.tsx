import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { PermissionsStep } from '../view/onboarding/PermissionsStep';
import { invoke } from '../testUtils/mocks/tauri';

describe('OnboardingView', () => {
  beforeEach(() => {
    invoke.mockClear();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  /**
   * Set up invoke mock for the standard permission check commands.
   * - accessibility: whether check_accessibility_permission returns true
   * - inputMonitoring: whether check_input_monitoring_permission returns true
   * - screenRecording: whether check_screen_recording_tcc_granted returns true
   */
  function setupPermissions(
    accessibility: boolean,
    inputMonitoring = false,
    screenRecording = false,
  ) {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return accessibility;
      if (cmd === 'check_input_monitoring_permission') return inputMonitoring;
      if (cmd === 'check_screen_recording_permission') return screenRecording;
      if (cmd === 'check_screen_recording_tcc_granted') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'request_input_monitoring_access') return;
      if (cmd === 'open_input_monitoring_settings') return;
    });
  }

  // ─── Basic render ──────────────────────────────────────────────────────────

  it('shows step 1 as active when accessibility is not granted', async () => {
    setupPermissions(false);
    render(<PermissionsStep />);
    await act(async () => {});

    expect(screen.getByText('Accessibility')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: /grant accessibility/i }),
    ).toBeInTheDocument();
  });

  it('shows the onboarding title', async () => {
    setupPermissions(false);
    render(<PermissionsStep />);
    await act(async () => {});

    expect(screen.getByText("Let's get Thuki set up")).toBeInTheDocument();
  });

  it('shows all three steps regardless of current active step', async () => {
    setupPermissions(false);
    render(<PermissionsStep />);
    await act(async () => {});

    expect(screen.getByText('Accessibility')).toBeInTheDocument();
    expect(screen.getByText('Input Monitoring')).toBeInTheDocument();
    expect(screen.getByText('Screen Recording')).toBeInTheDocument();
  });

  // ─── Step 1: Accessibility ─────────────────────────────────────────────────

  it('clicking grant accessibility invokes open settings command', async () => {
    setupPermissions(false);
    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    expect(invoke).toHaveBeenCalledWith('open_accessibility_settings');
  });

  it('shows spinner while polling after accessibility grant request', async () => {
    setupPermissions(false);
    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    const btn = screen.getByRole('button', {
      name: /checking|grant accessibility/i,
    });
    expect(btn).toBeDisabled();
  });

  it('keeps polling when accessibility not yet granted on first poll interval', async () => {
    let accessibilityGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return accessibilityGranted;
      if (cmd === 'check_input_monitoring_permission') return false;
      if (cmd === 'open_accessibility_settings') return;
    });

    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    // Still on step 1, input monitoring button not yet shown
    expect(
      screen.queryByRole('button', { name: /grant input monitoring/i }),
    ).toBeNull();

    accessibilityGranted = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    // Step 2 now active
    expect(
      screen.getByRole('button', { name: /grant input monitoring/i }),
    ).toBeInTheDocument();
  });

  it('advances to step 2 (input monitoring) when polling detects accessibility granted', async () => {
    let accessibilityGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return accessibilityGranted;
      if (cmd === 'check_input_monitoring_permission') return false;
      if (cmd === 'open_accessibility_settings') return;
    });

    render(<PermissionsStep />);
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

    expect(
      screen.getByRole('button', { name: /grant input monitoring/i }),
    ).toBeInTheDocument();
  });

  it('step 1 shows granted badge after accessibility is detected', async () => {
    let accessibilityGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return accessibilityGranted;
      if (cmd === 'check_input_monitoring_permission') return false;
      if (cmd === 'open_accessibility_settings') return;
    });

    render(<PermissionsStep />);
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

  // ─── Mount: skip completed steps ──────────────────────────────────────────

  it('skips to step 2 when accessibility is already granted on mount', async () => {
    setupPermissions(true, false);
    render(<PermissionsStep />);
    await act(async () => {});

    expect(
      screen.queryByRole('button', { name: /grant accessibility/i }),
    ).toBeNull();
    expect(
      screen.getByRole('button', { name: /grant input monitoring/i }),
    ).toBeInTheDocument();
  });

  it('skips to step 3 when accessibility and input monitoring are both granted on mount', async () => {
    setupPermissions(true, true);
    render(<PermissionsStep />);
    await act(async () => {});

    expect(
      screen.queryByRole('button', { name: /grant accessibility/i }),
    ).toBeNull();
    expect(
      screen.queryByRole('button', { name: /grant input monitoring/i }),
    ).toBeNull();
    expect(
      screen.getByRole('button', { name: /open screen recording settings/i }),
    ).toBeInTheDocument();
  });

  // ─── Step 2: Input Monitoring ──────────────────────────────────────────────

  it('clicking grant input monitoring invokes request and open-settings commands', async () => {
    setupPermissions(true, false);
    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });

    expect(invoke).toHaveBeenCalledWith('request_input_monitoring_access');
    expect(invoke).toHaveBeenCalledWith('open_input_monitoring_settings');
  });

  it('shows spinner while polling after input monitoring grant request', async () => {
    setupPermissions(true, false);
    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });

    const btn = screen.getByRole('button', {
      name: /checking|grant input monitoring/i,
    });
    expect(btn).toBeDisabled();
  });

  it('keeps polling when input monitoring not yet granted on first poll interval', async () => {
    let imGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_input_monitoring_permission') return imGranted;
      if (cmd === 'request_input_monitoring_access') return;
      if (cmd === 'open_input_monitoring_settings') return;
    });

    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    // Still on step 2, screen recording button not yet shown
    expect(
      screen.queryByRole('button', { name: /open screen recording settings/i }),
    ).toBeNull();

    imGranted = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(
      screen.getByRole('button', { name: /open screen recording settings/i }),
    ).toBeInTheDocument();
  });

  it('advances to step 3 when polling detects input monitoring granted', async () => {
    let imGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_input_monitoring_permission') return imGranted;
      if (cmd === 'request_input_monitoring_access') return;
      if (cmd === 'open_input_monitoring_settings') return;
    });

    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });

    imGranted = true;

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(
      screen.getByRole('button', { name: /open screen recording settings/i }),
    ).toBeInTheDocument();
  });

  it('step 2 shows granted badge after input monitoring is detected', async () => {
    let imGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_input_monitoring_permission') return imGranted;
      if (cmd === 'request_input_monitoring_access') return;
      if (cmd === 'open_input_monitoring_settings') return;
    });

    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });

    imGranted = true;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    const badges = screen.getAllByText('Granted');
    expect(badges.length).toBeGreaterThanOrEqual(2);
  });

  // ─── Step 3: Screen Recording ──────────────────────────────────────────────

  it('clicking open screen recording settings registers app and opens settings', async () => {
    setupPermissions(true, true);
    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    expect(invoke).toHaveBeenCalledWith('request_screen_recording_access');
    expect(invoke).toHaveBeenCalledWith('open_screen_recording_settings');
  });

  it('shows spinner while polling after opening screen recording settings', async () => {
    setupPermissions(true, true);
    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    const btn = screen.getByRole('button', {
      name: /checking|open screen recording settings/i,
    });
    expect(btn).toBeDisabled();
  });

  it('does not show quit and reopen immediately after clicking screen recording button', async () => {
    setupPermissions(true, true);
    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    expect(screen.queryByRole('button', { name: /quit.*reopen/i })).toBeNull();
  });

  it('keeps polling when screen recording tcc not yet granted', async () => {
    let tccGranted = false;
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_input_monitoring_permission') return true;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'check_screen_recording_tcc_granted') return tccGranted;
    });

    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(500);
    });

    expect(screen.queryByRole('button', { name: /quit.*reopen/i })).toBeNull();

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
      if (cmd === 'check_input_monitoring_permission') return true;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'check_screen_recording_tcc_granted') return true;
    });

    render(<PermissionsStep />);
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
      if (cmd === 'check_input_monitoring_permission') return true;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'check_screen_recording_tcc_granted') return true;
    });

    render(<PermissionsStep />);
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

  // ─── Unmount cleanup ──────────────────────────────────────────────────────

  it('does not emit console.error when unmounted during accessibility polling', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

    setupPermissions(false);
    const { unmount } = render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    act(() => unmount());

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('does not emit console.error when unmounted during input monitoring polling', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'check_accessibility_permission') return true;
      if (cmd === 'check_input_monitoring_permission') return false;
      if (cmd === 'request_input_monitoring_access') return;
      if (cmd === 'open_input_monitoring_settings') return;
    });

    const { unmount } = render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });

    act(() => unmount());

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
      if (cmd === 'check_input_monitoring_permission') return true;
      if (cmd === 'check_screen_recording_permission') return false;
      if (cmd === 'request_screen_recording_access') return;
      if (cmd === 'open_screen_recording_settings') return;
      if (cmd === 'check_screen_recording_tcc_granted') return false;
    });

    const { unmount } = render(<PermissionsStep />);
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

  // ─── CTAButton hover ───────────────────────────────────────────────────────

  it('hovering the CTA button applies brightness filter when enabled', async () => {
    setupPermissions(false);
    render(<PermissionsStep />);
    await act(async () => {});

    const btn = screen.getByRole('button', { name: /grant accessibility/i });
    fireEvent.mouseEnter(btn);
    expect(btn).toBeInTheDocument();
    fireEvent.mouseLeave(btn);
    expect(btn).toBeInTheDocument();
  });

  it('hovering a disabled CTA button does not apply brightness filter', async () => {
    setupPermissions(false);
    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    const btn = screen.getByRole('button', {
      name: /checking|grant accessibility/i,
    });
    expect(btn).toBeDisabled();
    fireEvent.mouseEnter(btn);
    expect(btn).toBeDisabled();
    fireEvent.mouseLeave(btn);
    expect(btn).toBeDisabled();
  });

  // ─── Defensive guard coverage ─────────────────────────────────────────────
  // Tests below exercise the early-return branches that protect against stale
  // state updates and concurrent invocations. Deferred promises keep invocations
  // in-flight long enough to trigger each guard before resolving them.

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

    const { unmount } = render(<PermissionsStep />);
    // useEffect has fired; initial invoke is in-flight (resolveInitial is set).

    act(() => unmount()); // mountedRef → false

    await act(async () => {
      resolveInitial(true); // then-handler fires; guard returns early
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('ignores initial input monitoring check result when component unmounts mid-flight', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    let resolveIm!: (v: boolean) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission')
        return Promise.resolve(true);
      if (cmd === 'check_input_monitoring_permission')
        return new Promise((r) => {
          resolveIm = r;
        }); // hangs
      return Promise.resolve();
    });

    const { unmount } = render(<PermissionsStep />);
    // ax check resolves true; im check is now in-flight.
    await act(async () => {});

    act(() => unmount()); // mountedRef → false

    await act(async () => {
      resolveIm(true); // then-handler fires; guard returns early
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

    render(<PermissionsStep />);
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
    // tick was blocked, no count=3).
    expect(pollCallCount).toBe(2);

    await act(async () => {
      resolveFirstPoll(false);
    });
  });

  it('im in-flight guard prevents concurrent permission checks', async () => {
    let imCallCount = 0;
    let resolveFirstPoll!: (v: boolean) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission')
        return Promise.resolve(true);
      if (cmd === 'check_input_monitoring_permission') {
        imCallCount++;
        if (imCallCount === 1) return Promise.resolve(false); // initial mount check
        return new Promise((r) => {
          resolveFirstPoll = r;
        }); // poll hangs
      }
      if (cmd === 'request_input_monitoring_access') return Promise.resolve();
      if (cmd === 'open_input_monitoring_settings') return Promise.resolve();
      return Promise.resolve();
    });

    render(<PermissionsStep />);
    await act(async () => {}); // ax + initial im check done

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });

    act(() => {
      vi.advanceTimersByTime(500); // first tick: in-flight
      vi.advanceTimersByTime(500); // second tick: guard blocks it
    });

    expect(imCallCount).toBe(2); // initial check + one poll; second tick blocked

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

    const { unmount } = render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant accessibility/i }),
      );
    });

    act(() => vi.advanceTimersByTime(500));

    act(() => unmount());

    await act(async () => {
      resolvePoll(true);
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('ignores im poll result when component unmounts during in-flight check', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    let imCallCount = 0;
    let resolvePoll!: (v: boolean) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission')
        return Promise.resolve(true);
      if (cmd === 'check_input_monitoring_permission') {
        imCallCount++;
        if (imCallCount === 1) return Promise.resolve(false); // initial mount check
        return new Promise((r) => {
          resolvePoll = r;
        });
      }
      if (cmd === 'request_input_monitoring_access') return Promise.resolve();
      if (cmd === 'open_input_monitoring_settings') return Promise.resolve();
      return Promise.resolve();
    });

    const { unmount } = render(<PermissionsStep />);
    await act(async () => {}); // ax + initial im check done

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });

    act(() => vi.advanceTimersByTime(500)); // poll fires, invoke hangs

    act(() => unmount()); // clears interval; in-flight promise still alive

    await act(async () => {
      resolvePoll(true);
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });

  it('ignores input monitoring handler when component unmounts during open-settings call', async () => {
    const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
    let resolveOpen!: (v?: unknown) => void;
    invoke.mockImplementation((cmd: string) => {
      if (cmd === 'check_accessibility_permission')
        return Promise.resolve(true);
      if (cmd === 'check_input_monitoring_permission')
        return Promise.resolve(false);
      if (cmd === 'request_input_monitoring_access') return Promise.resolve();
      if (cmd === 'open_input_monitoring_settings')
        return new Promise((r) => {
          resolveOpen = r;
        }); // hangs
      return Promise.resolve();
    });

    const { unmount } = render(<PermissionsStep />);
    await act(async () => {}); // ax granted; im check done (false)

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /grant input monitoring/i }),
      );
    });
    // handler is suspended on open_input_monitoring_settings (resolveOpen set)

    act(() => unmount()); // mountedRef → false

    await act(async () => {
      resolveOpen(); // mountedRef guard fires; returns early
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
      if (cmd === 'check_input_monitoring_permission')
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

    const { unmount } = render(<PermissionsStep />);
    await act(async () => {}); // ax + im both granted

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    act(() => unmount()); // mountedRef → false

    await act(async () => {
      resolveOpen(); // mountedRef guard fires; returns early
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
      if (cmd === 'check_input_monitoring_permission')
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

    render(<PermissionsStep />);
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
      if (cmd === 'check_input_monitoring_permission')
        return Promise.resolve(true);
      if (cmd === 'request_screen_recording_access') return Promise.resolve();
      if (cmd === 'open_screen_recording_settings') return Promise.resolve();
      if (cmd === 'check_screen_recording_tcc_granted')
        return new Promise((r) => {
          resolvePoll = r;
        });
      return Promise.resolve();
    });

    const { unmount } = render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: /open screen recording settings/i }),
      );
    });

    act(() => vi.advanceTimersByTime(500)); // poll fires, invoke hangs

    act(() => unmount()); // clears interval; in-flight promise still alive

    await act(async () => {
      resolvePoll(true);
    });

    expect(errorSpy).not.toHaveBeenCalled();
    errorSpy.mockRestore();
  });
});
