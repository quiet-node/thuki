import { render, screen, fireEvent, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { PermissionsStep } from '../PermissionsStep';
import {
  invoke,
  enableChannelCaptureWithResponses,
} from '../../../testUtils/mocks/tauri';

const BASE_RESPONSES = {
  consume_pending_grant_resume: null,
  check_accessibility_permission: false,
};

describe('PermissionsStep', () => {
  beforeEach(() => {
    invoke.mockClear();
  });

  it('renders the title and the Accessibility step as active, Screen Recording as waiting', async () => {
    enableChannelCaptureWithResponses(BASE_RESPONSES);

    render(<PermissionsStep />);
    await act(async () => {});

    expect(screen.getByText("Let's get Thuki set up")).toBeInTheDocument();
    expect(screen.getByText('Accessibility')).toBeInTheDocument();
    expect(
      screen.getByText('Needed for /screen to capture your entire screen'),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Grant Accessibility Access' }),
    ).toBeInTheDocument();
    expect(screen.queryByText('Granted')).not.toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Open Screen Recording Settings' }),
    ).not.toBeInTheDocument();
  });

  it('brightens the CTA on hover and reverts on mouse leave', async () => {
    enableChannelCaptureWithResponses(BASE_RESPONSES);

    render(<PermissionsStep />);
    await act(async () => {});

    const button = screen.getByRole('button', {
      name: 'Grant Accessibility Access',
    });
    fireEvent.mouseEnter(button);
    expect(button.style.filter).toBe('brightness(1.1)');

    fireEvent.mouseLeave(button);
    expect(button.style.filter).toBe('none');
  });

  it('does not brighten the CTA on hover while it is disabled', async () => {
    // Once a grant flow is in flight the button stays disabled ("Checking...")
    // until polling detects the grant, so hovering it must not brighten it.
    enableChannelCaptureWithResponses({
      ...BASE_RESPONSES,
      reset_and_relaunch_for_grant: false,
    });

    render(<PermissionsStep />);
    await act(async () => {});

    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: 'Grant Accessibility Access' }),
      );
    });

    const button = screen.getByRole('button', { name: 'Checking...' });
    fireEvent.mouseEnter(button);
    expect(button.style.filter).toBe('none');
  });

  it('shows Accessibility as done and Screen Recording as active when already granted on mount', async () => {
    enableChannelCaptureWithResponses({
      ...BASE_RESPONSES,
      check_accessibility_permission: true,
    });

    render(<PermissionsStep />);
    await act(async () => {});

    expect(screen.getByText('Granted')).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Open Screen Recording Settings' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: 'Grant Accessibility Access' }),
    ).not.toBeInTheDocument();
  });

  it('requests accessibility settings when the grant button is clicked (no relaunch needed)', async () => {
    enableChannelCaptureWithResponses({
      ...BASE_RESPONSES,
      reset_and_relaunch_for_grant: false,
    });

    render(<PermissionsStep />);
    await act(async () => {});

    const button = screen.getByRole('button', {
      name: 'Grant Accessibility Access',
    });
    await act(async () => {
      fireEvent.click(button);
    });

    expect(invoke).toHaveBeenCalledWith('reset_and_relaunch_for_grant', {
      service: 'Accessibility',
    });
    expect(invoke).toHaveBeenCalledWith('open_accessibility_settings');
    expect(
      screen.getByRole('button', { name: 'Checking...' }),
    ).toBeInTheDocument();
  });

  it('does not open settings when reset_and_relaunch_for_grant signals a relaunch is in progress', async () => {
    enableChannelCaptureWithResponses({
      ...BASE_RESPONSES,
      reset_and_relaunch_for_grant: true,
    });

    render(<PermissionsStep />);
    await act(async () => {});

    const button = screen.getByRole('button', {
      name: 'Grant Accessibility Access',
    });
    await act(async () => {
      fireEvent.click(button);
    });

    expect(invoke).toHaveBeenCalledWith('reset_and_relaunch_for_grant', {
      service: 'Accessibility',
    });
    expect(invoke).not.toHaveBeenCalledWith('open_accessibility_settings');
  });

  describe('polling flows', () => {
    beforeEach(() => {
      vi.useFakeTimers();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('advances Accessibility to granted once polling detects the permission', async () => {
      let accessibilityGranted = false;
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'consume_pending_grant_resume') return null;
        if (cmd === 'check_accessibility_permission')
          return accessibilityGranted;
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'open_accessibility_settings') return undefined;
        return undefined;
      });

      render(<PermissionsStep />);
      await act(async () => {});

      const button = screen.getByRole('button', {
        name: 'Grant Accessibility Access',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      // First tick still finds it ungranted, exercising the "not yet" branch.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(
        screen.getByRole('button', { name: 'Checking...' }),
      ).toBeInTheDocument();

      accessibilityGranted = true;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });

      expect(screen.getByText('Granted')).toBeInTheDocument();
      expect(
        screen.getByRole('button', { name: 'Open Screen Recording Settings' }),
      ).toBeInTheDocument();
    });

    it('does not open Screen Recording settings when reset_and_relaunch_for_grant signals a relaunch is in progress', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'consume_pending_grant_resume') return null;
        if (cmd === 'check_accessibility_permission') return true;
        if (cmd === 'reset_and_relaunch_for_grant') return true;
        return undefined;
      });

      render(<PermissionsStep />);
      await act(async () => {});

      const button = screen.getByRole('button', {
        name: 'Open Screen Recording Settings',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      expect(invoke).toHaveBeenCalledWith('reset_and_relaunch_for_grant', {
        service: 'ScreenCapture',
      });
      expect(invoke).not.toHaveBeenCalledWith(
        'request_screen_recording_access',
      );
    });

    it('requests screen recording access and settings when its CTA is clicked', async () => {
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'consume_pending_grant_resume') return null;
        if (cmd === 'check_accessibility_permission') return true;
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'request_screen_recording_access') return undefined;
        if (cmd === 'open_screen_recording_settings') return undefined;
        if (cmd === 'check_screen_recording_tcc_granted') return false;
        return undefined;
      });

      render(<PermissionsStep />);
      await act(async () => {});

      const button = screen.getByRole('button', {
        name: 'Open Screen Recording Settings',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      expect(invoke).toHaveBeenCalledWith('request_screen_recording_access');
      expect(invoke).toHaveBeenCalledWith('open_screen_recording_settings');
      expect(
        screen.getByRole('button', { name: 'Checking...' }),
      ).toBeInTheDocument();
    });

    it('shows Quit & Reopen once Screen Recording polling detects the permission, and it invokes quit_and_relaunch', async () => {
      let screenGranted = false;
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'consume_pending_grant_resume') return null;
        if (cmd === 'check_accessibility_permission') return true;
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'request_screen_recording_access') return undefined;
        if (cmd === 'open_screen_recording_settings') return undefined;
        if (cmd === 'check_screen_recording_tcc_granted') return screenGranted;
        if (cmd === 'quit_and_relaunch') return undefined;
        return undefined;
      });

      render(<PermissionsStep />);
      await act(async () => {});

      const openSettingsButton = screen.getByRole('button', {
        name: 'Open Screen Recording Settings',
      });
      await act(async () => {
        fireEvent.click(openSettingsButton);
      });

      // First tick still finds it ungranted, exercising the "not yet" branch.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(
        screen.getByRole('button', { name: 'Checking...' }),
      ).toBeInTheDocument();

      screenGranted = true;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });

      expect(
        screen.getByText(
          'macOS requires a restart for Screen Recording to take effect',
        ),
      ).toBeInTheDocument();

      const quitButton = screen.getByRole('button', {
        name: 'Quit and Reopen Thuki',
      });
      await act(async () => {
        fireEvent.click(quitButton);
      });

      expect(invoke).toHaveBeenCalledWith('quit_and_relaunch');
    });
  });

  describe('resume flow', () => {
    it('auto-resumes the Accessibility flow when the backend reports a pending Accessibility resume', async () => {
      enableChannelCaptureWithResponses({
        ...BASE_RESPONSES,
        consume_pending_grant_resume: 'Accessibility',
      });

      render(<PermissionsStep />);
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('open_accessibility_settings');
      expect(
        screen.getByRole('button', { name: 'Checking...' }),
      ).toBeInTheDocument();
    });

    it('auto-resumes the Screen Recording flow when the backend reports a pending ScreenCapture resume and Accessibility is already granted', async () => {
      enableChannelCaptureWithResponses({
        consume_pending_grant_resume: 'ScreenCapture',
        check_accessibility_permission: true,
      });

      render(<PermissionsStep />);
      await act(async () => {});

      expect(invoke).toHaveBeenCalledWith('request_screen_recording_access');
      expect(invoke).toHaveBeenCalledWith('open_screen_recording_settings');
    });

    it('does not auto-resume the Screen Recording flow when Accessibility is not yet granted', async () => {
      enableChannelCaptureWithResponses({
        consume_pending_grant_resume: 'ScreenCapture',
        check_accessibility_permission: false,
      });

      render(<PermissionsStep />);
      await act(async () => {});

      expect(invoke).not.toHaveBeenCalledWith(
        'request_screen_recording_access',
      );
    });
  });

  describe('unmount guards', () => {
    // These exercise the mountedRef.current === false branches: an invoke()
    // call resolves after the component has already unmounted, and the
    // resulting state update / follow-on call must be skipped rather than
    // throwing on an unmounted component.

    afterEach(() => {
      vi.useRealTimers();
    });

    it('skips the mount-effect state updates when unmounted before consume_pending_grant_resume resolves', async () => {
      let resolveResume: (value: string | null) => void = () => {};
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'consume_pending_grant_resume') {
          return new Promise<string | null>((resolve) => {
            resolveResume = resolve;
          });
        }
        return undefined;
      });

      const { unmount } = render(<PermissionsStep />);
      unmount();
      await act(async () => {
        resolveResume(null);
        await Promise.resolve();
      });

      // If the guard were missing, the mount effect would fall through to
      // its next line regardless of what `resume` resolved to.
      expect(invoke).not.toHaveBeenCalledWith('check_accessibility_permission');
    });

    it('skips the mount-effect state updates when unmounted before check_accessibility_permission resolves', async () => {
      let resolveCheck: (value: boolean) => void = () => {};
      // resume is 'Accessibility' so that, if the guard below were missing,
      // the effect would fall through to auto-starting startAccessibilityFlow
      // (and thus invoke open_accessibility_settings) regardless of what
      // check_accessibility_permission resolves to — giving the guard an
      // observable effect to assert against.
      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'consume_pending_grant_resume') return 'Accessibility';
        if (cmd === 'check_accessibility_permission') {
          return new Promise<boolean>((resolve) => {
            resolveCheck = resolve;
          });
        }
        return undefined;
      });

      const { unmount } = render(<PermissionsStep />);
      await act(async () => {});
      unmount();
      await act(async () => {
        resolveCheck(true);
        await Promise.resolve();
      });

      expect(invoke).not.toHaveBeenCalledWith('open_accessibility_settings');
    });

    it('skips starting Accessibility polling when unmounted right after settings succeed', async () => {
      let resolveOpenSettings: () => void = () => {};
      enableChannelCaptureWithResponses(BASE_RESPONSES);
      vi.useFakeTimers();

      const { unmount } = render(<PermissionsStep />);
      await act(async () => {});

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'open_accessibility_settings') {
          return new Promise<void>((resolve) => {
            resolveOpenSettings = resolve;
          });
        }
        return undefined;
      });

      const button = screen.getByRole('button', {
        name: 'Grant Accessibility Access',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      unmount();
      await act(async () => {
        resolveOpenSettings();
        await Promise.resolve();
      });

      // Clear the mount effect's own earlier check_accessibility_permission
      // call so the assertion below only reflects calls made from here on.
      invoke.mockClear();

      // If the guard were missing, setInterval would have been scheduled;
      // advancing past a full tick and seeing no poll call proves it wasn't.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(invoke).not.toHaveBeenCalledWith('check_accessibility_permission');
    });

    it('skips a poll tick while the previous Accessibility poll invoke is still in flight', async () => {
      let resolveFirstCheck: (value: boolean) => void = () => {};
      let checkCallCount = 0;
      enableChannelCaptureWithResponses(BASE_RESPONSES);
      vi.useFakeTimers();

      render(<PermissionsStep />);
      await act(async () => {});

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'open_accessibility_settings') return undefined;
        if (cmd === 'check_accessibility_permission') {
          checkCallCount += 1;
          if (checkCallCount === 1) {
            return new Promise<boolean>((resolve) => {
              resolveFirstCheck = resolve;
            });
          }
          return true;
        }
        return undefined;
      });

      const button = screen.getByRole('button', {
        name: 'Grant Accessibility Access',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      // First tick starts and never resolves yet; the second tick fires
      // while it's still in flight and must be skipped by the guard.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1000);
      });
      expect(checkCallCount).toBe(1);

      await act(async () => {
        resolveFirstCheck(false);
        await Promise.resolve();
      });
    });

    it('skips a poll-tick state update when unmounted while check_accessibility_permission is in flight', async () => {
      let resolveGrantedCheck: (value: boolean) => void = () => {};
      let firstTickStarted = false;
      enableChannelCaptureWithResponses(BASE_RESPONSES);
      vi.useFakeTimers();

      const { unmount } = render(<PermissionsStep />);
      await act(async () => {});

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'open_accessibility_settings') return undefined;
        if (cmd === 'check_accessibility_permission') {
          firstTickStarted = true;
          return new Promise<boolean>((resolve) => {
            resolveGrantedCheck = resolve;
          });
        }
        return undefined;
      });

      const button = screen.getByRole('button', {
        name: 'Grant Accessibility Access',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(firstTickStarted).toBe(true);

      unmount();
      // If this guard were missing, `granted` resolving true would run
      // `stopAxPolling()` (a harmless no-op here, since unmount's own
      // cleanup already cleared the interval) and `setAccessibilityStatus`
      // on a gone component. Neither has a further invoke() call to assert
      // against, so "resolving this after unmount does not throw" is the
      // strongest assertion available for this specific guard.
      await act(async () => {
        resolveGrantedCheck(true);
        await Promise.resolve();
      });
    });

    it('skips resetting Accessibility status when unmounted before reset_and_relaunch_for_grant resolves', async () => {
      let resolveReset: (value: boolean) => void = () => {};
      enableChannelCaptureWithResponses(BASE_RESPONSES);

      const { unmount } = render(<PermissionsStep />);
      await act(async () => {});

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'reset_and_relaunch_for_grant') {
          return new Promise<boolean>((resolve) => {
            resolveReset = resolve;
          });
        }
        return undefined;
      });

      const button = screen.getByRole('button', {
        name: 'Grant Accessibility Access',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      unmount();
      await act(async () => {
        resolveReset(false);
        await Promise.resolve();
      });

      expect(invoke).not.toHaveBeenCalledWith('open_accessibility_settings');
    });

    it('skips starting Screen Recording polling when unmounted right after settings succeed', async () => {
      let resolveOpenSettings: () => void = () => {};
      enableChannelCaptureWithResponses({
        ...BASE_RESPONSES,
        check_accessibility_permission: true,
      });
      vi.useFakeTimers();

      const { unmount } = render(<PermissionsStep />);
      await act(async () => {});

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'request_screen_recording_access') return undefined;
        if (cmd === 'open_screen_recording_settings') {
          return new Promise<void>((resolve) => {
            resolveOpenSettings = resolve;
          });
        }
        return undefined;
      });

      const button = screen.getByRole('button', {
        name: 'Open Screen Recording Settings',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      unmount();
      await act(async () => {
        resolveOpenSettings();
        await Promise.resolve();
      });

      // If the guard were missing, setInterval would have been scheduled;
      // advancing past a full tick and seeing no poll call proves it wasn't.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(invoke).not.toHaveBeenCalledWith(
        'check_screen_recording_tcc_granted',
      );
    });

    it('skips a poll tick while the previous Screen Recording poll invoke is still in flight', async () => {
      let resolveFirstCheck: (value: boolean) => void = () => {};
      let checkCallCount = 0;
      enableChannelCaptureWithResponses({
        ...BASE_RESPONSES,
        check_accessibility_permission: true,
      });
      vi.useFakeTimers();

      render(<PermissionsStep />);
      await act(async () => {});

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'request_screen_recording_access') return undefined;
        if (cmd === 'open_screen_recording_settings') return undefined;
        if (cmd === 'check_screen_recording_tcc_granted') {
          checkCallCount += 1;
          if (checkCallCount === 1) {
            return new Promise<boolean>((resolve) => {
              resolveFirstCheck = resolve;
            });
          }
          return true;
        }
        return undefined;
      });

      const button = screen.getByRole('button', {
        name: 'Open Screen Recording Settings',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      // First tick starts and never resolves yet; the second tick fires
      // while it's still in flight and must be skipped by the guard.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1000);
      });
      expect(checkCallCount).toBe(1);

      await act(async () => {
        resolveFirstCheck(false);
        await Promise.resolve();
      });
    });

    it('skips a poll-tick state update when unmounted while check_screen_recording_tcc_granted is in flight', async () => {
      let resolveGrantedCheck: (value: boolean) => void = () => {};
      let firstTickStarted = false;
      enableChannelCaptureWithResponses({
        ...BASE_RESPONSES,
        check_accessibility_permission: true,
      });
      vi.useFakeTimers();

      const { unmount } = render(<PermissionsStep />);
      await act(async () => {});

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'reset_and_relaunch_for_grant') return false;
        if (cmd === 'request_screen_recording_access') return undefined;
        if (cmd === 'open_screen_recording_settings') return undefined;
        if (cmd === 'check_screen_recording_tcc_granted') {
          firstTickStarted = true;
          return new Promise<boolean>((resolve) => {
            resolveGrantedCheck = resolve;
          });
        }
        return undefined;
      });

      const button = screen.getByRole('button', {
        name: 'Open Screen Recording Settings',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(firstTickStarted).toBe(true);

      unmount();
      // If this guard were missing, `granted` resolving true would run
      // `stopScreenPolling()` (a harmless no-op here, since unmount's own
      // cleanup already cleared the interval) and `setScreenRecordingStatus`
      // on a gone component. Neither has a further invoke() call to assert
      // against, so "resolving this after unmount does not throw" is the
      // strongest assertion available for this specific guard.
      await act(async () => {
        resolveGrantedCheck(true);
        await Promise.resolve();
      });
    });

    it('skips resetting Screen Recording status when unmounted before reset_and_relaunch_for_grant resolves', async () => {
      let resolveReset: (value: boolean) => void = () => {};
      enableChannelCaptureWithResponses({
        ...BASE_RESPONSES,
        check_accessibility_permission: true,
      });

      const { unmount } = render(<PermissionsStep />);
      await act(async () => {});

      invoke.mockImplementation(async (cmd: string) => {
        if (cmd === 'reset_and_relaunch_for_grant') {
          return new Promise<boolean>((resolve) => {
            resolveReset = resolve;
          });
        }
        return undefined;
      });

      const button = screen.getByRole('button', {
        name: 'Open Screen Recording Settings',
      });
      await act(async () => {
        fireEvent.click(button);
      });

      unmount();
      await act(async () => {
        resolveReset(false);
        await Promise.resolve();
      });

      expect(invoke).not.toHaveBeenCalledWith(
        'request_screen_recording_access',
      );
    });
  });
});
