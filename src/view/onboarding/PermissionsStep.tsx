import { motion } from 'framer-motion';
import type React from 'react';
import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import thukiLogo from '../../../src-tauri/icons/128x128.png';
import { StepCard, Badge } from './_shared';

/** How often to poll for permission grants after the user requests them. */
const POLL_INTERVAL_MS = 500;

type AccessibilityStatus = 'pending' | 'requesting' | 'granted';
type ScreenRecordingStatus = 'idle' | 'polling' | 'granted';

/** Inline macOS-style keyboard key chip for showing hotkey symbols. */
const KeyChip = ({ label }: { label: string }) => (
  <span
    style={{
      display: 'inline-flex',
      alignItems: 'center',
      justifyContent: 'center',
      padding: '1px 5px',
      background: 'rgba(255,255,255,0.08)',
      border: '1px solid rgba(255,255,255,0.18)',
      borderBottom: '2px solid rgba(255,255,255,0.12)',
      borderRadius: 4,
      fontSize: 11,
      lineHeight: 1.4,
      color: 'rgba(255,255,255,0.75)',
      verticalAlign: 'middle',
      margin: '0 1px',
      fontFamily: 'inherit',
    }}
  >
    {label}
  </span>
);

/** Checkmark icon for the granted step state. */
const CheckIcon = () => (
  <svg
    width="18"
    height="18"
    viewBox="0 0 18 18"
    fill="none"
    aria-hidden="true"
  >
    <path
      d="M4 9l3.5 3.5 7-7"
      stroke="#22c55e"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    />
  </svg>
);

/** Keyboard/accessibility icon for the active step 1. */
const KeyboardIcon = () => (
  <svg
    width="18"
    height="18"
    viewBox="0 0 18 18"
    fill="none"
    aria-hidden="true"
  >
    <rect
      x="2"
      y="4"
      width="14"
      height="10"
      rx="2"
      stroke="#ff8d5c"
      strokeWidth="1.5"
    />
    <path
      d="M5 8h1M8 8h1M11 8h1M5 11h8"
      stroke="#ff8d5c"
      strokeWidth="1.5"
      strokeLinecap="round"
    />
  </svg>
);

/** Screen/camera icon for step 2. */
const ScreenIcon = ({ active }: { active: boolean }) => (
  <svg
    width="18"
    height="18"
    viewBox="0 0 18 18"
    fill="none"
    aria-hidden="true"
  >
    <rect
      x="2"
      y="5"
      width="14"
      height="9"
      rx="2"
      stroke={active ? '#ff8d5c' : '#6b6660'}
      strokeWidth="1.5"
    />
    <circle cx="9" cy="9.5" r="2" fill={active ? '#ff8d5c' : '#6b6660'} />
    <circle
      cx="9"
      cy="9.5"
      r="3.5"
      stroke={active ? '#ff8d5c' : '#6b6660'}
      strokeWidth="0.8"
      opacity="0.4"
    />
  </svg>
);

/** Minimal animated spinner. */
const Spinner = () => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 16 16"
    fill="none"
    aria-label="Checking..."
    style={{ animation: 'spin 0.8s linear infinite' }}
  >
    <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
    <circle
      cx="8"
      cy="8"
      r="6"
      stroke="rgba(255,255,255,0.2)"
      strokeWidth="2"
    />
    <path
      d="M8 2a6 6 0 0 1 6 6"
      stroke="white"
      strokeWidth="2"
      strokeLinecap="round"
    />
  </svg>
);

/**
 * Onboarding screen shown at first launch when required macOS permissions
 * (Accessibility and Screen Recording) have not yet been granted.
 *
 * Follows a sequential flow: Accessibility first (polls until granted,
 * no restart needed), then Screen Recording (registers app via
 * CGRequestScreenCaptureAccess, polls TCC until granted, then prompts
 * quit+reopen since macOS requires a restart for the permission to take effect).
 *
 * Visual direction: Warm Ambient: dark base with a warm orange radial glow.
 * The outer container is transparent so the rounded panel corners are visible
 * against the macOS desktop.
 */
export function PermissionsStep() {
  const [accessibilityStatus, setAccessibilityStatus] =
    useState<AccessibilityStatus>('pending');
  const [screenRecordingStatus, setScreenRecordingStatus] =
    useState<ScreenRecordingStatus>('idle');
  const axPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const screenPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  // Guards that prevent a new poll tick from firing while a previous invoke
  // call is still in-flight. Without these, a slow IPC response (> POLL_INTERVAL_MS)
  // could queue multiple concurrent permission checks.
  const axInFlightRef = useRef(false);
  const screenInFlightRef = useRef(false);
  // Prevents state updates from resolving in-flight invocations after unmount.
  const mountedRef = useRef(true);

  const stopAxPolling = useCallback(() => {
    if (axPollRef.current !== null) {
      clearInterval(axPollRef.current);
      axPollRef.current = null;
    }
  }, []);

  const stopScreenPolling = useCallback(() => {
    if (screenPollRef.current !== null) {
      clearInterval(screenPollRef.current);
      screenPollRef.current = null;
    }
  }, []);

  // Inline grant flow used both by a fresh click (when the backend reports
  // no relaunch needed) and by the post-restart resume path. Opens System
  // Settings directly and polls AXIsProcessTrusted until the user toggles
  // the permission on. The first AXIsProcessTrusted call from a fresh PID
  // is what registers Thuki in the System Settings list, so polling does
  // double duty here.
  const startAccessibilityFlow = useCallback(async () => {
    setAccessibilityStatus('requesting');
    await invoke('open_accessibility_settings');
    if (!mountedRef.current) return;
    axPollRef.current = setInterval(async () => {
      if (axInFlightRef.current) return;
      axInFlightRef.current = true;
      try {
        const granted = await invoke<boolean>('check_accessibility_permission');
        if (!mountedRef.current) return;
        if (granted) {
          stopAxPolling();
          setAccessibilityStatus('granted');
        }
      } finally {
        axInFlightRef.current = false;
      }
    }, POLL_INTERVAL_MS);
  }, [stopAxPolling]);

  const startScreenRecordingFlow = useCallback(async () => {
    // CGRequestScreenCaptureAccess is the call that adds Thuki to the
    // Screen Recording list AND surfaces the macOS allow dialog. Without
    // it the entry never appears, so the user has nothing to toggle.
    await invoke('request_screen_recording_access');
    await invoke('open_screen_recording_settings');
    if (!mountedRef.current) return;
    setScreenRecordingStatus('polling');
    screenPollRef.current = setInterval(async () => {
      if (screenInFlightRef.current) return;
      screenInFlightRef.current = true;
      try {
        const granted = await invoke<boolean>(
          'check_screen_recording_tcc_granted',
        );
        if (!mountedRef.current) return;
        if (granted) {
          stopScreenPolling();
          setScreenRecordingStatus('granted');
        }
      } finally {
        screenInFlightRef.current = false;
      }
    }, POLL_INTERVAL_MS);
  }, [stopScreenPolling]);

  // On mount: drain any pending click-time resume marker the previous
  // process wrote before relaunching, then check whether Accessibility is
  // already granted so we can skip step 1 and show step 2 immediately.
  // Order matters: the resume handler may auto-start a flow whose state
  // would otherwise be clobbered by the granted-check setter.
  useEffect(() => {
    // Reset on every mount so that a remount after unmount gets a fresh guard.
    mountedRef.current = true;

    void (async () => {
      const resume = await invoke<string | null>(
        'consume_pending_grant_resume',
      );
      if (!mountedRef.current) return;

      const accessibilityGranted = await invoke<boolean>(
        'check_accessibility_permission',
      );
      if (!mountedRef.current) return;
      if (accessibilityGranted) setAccessibilityStatus('granted');

      if (resume === 'Accessibility') {
        // Previous process did the TCC reset+restart for Accessibility.
        // Resume the open-Settings + polling step so the user does not
        // have to click Grant a second time.
        void startAccessibilityFlow();
      } else if (resume === 'ScreenCapture' && accessibilityGranted) {
        void startScreenRecordingFlow();
      }
    })();

    return () => {
      mountedRef.current = false;
      stopAxPolling();
      stopScreenPolling();
    };
  }, [
    startAccessibilityFlow,
    startScreenRecordingFlow,
    stopAxPolling,
    stopScreenPolling,
  ]);

  const handleGrantAccessibility = useCallback(async () => {
    setAccessibilityStatus('requesting');
    // Backend clears any stale TCC.Accessibility entry left over from a
    // previous binary's code requirement and relaunches so tccd registers
    // the new csreq cleanly. When the startup path already did the reset
    // (true on a fresh install or a detected upgrade) the backend returns
    // false and we run the open-Settings flow inline without a relaunch.
    const restarting = await invoke<boolean>('reset_and_relaunch_for_grant', {
      service: 'Accessibility',
    });
    if (!mountedRef.current || restarting) return;
    await startAccessibilityFlow();
  }, [startAccessibilityFlow]);

  const handleOpenScreenRecording = useCallback(async () => {
    const restarting = await invoke<boolean>('reset_and_relaunch_for_grant', {
      service: 'ScreenCapture',
    });
    if (!mountedRef.current || restarting) return;
    await startScreenRecordingFlow();
  }, [startScreenRecordingFlow]);

  const handleQuitAndRelaunch = useCallback(async () => {
    await invoke('quit_and_relaunch');
  }, []);

  const accessibilityGranted = accessibilityStatus === 'granted';
  const isAxRequesting = accessibilityStatus === 'requesting';
  const isScreenPolling = screenRecordingStatus === 'polling';
  const screenGranted = screenRecordingStatus === 'granted';

  return (
    // Transparent outer container so the rounded panel corners show through
    // against the macOS desktop (window has transparent: true in tauri.conf.json).
    <div
      style={{
        minHeight: '100vh',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: 'transparent',
        fontFamily: 'inherit',
      }}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.97, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ type: 'spring', stiffness: 300, damping: 28 }}
        style={{
          width: 420,
          background:
            'radial-gradient(ellipse 80% 55% at 50% 0%, rgba(255,141,92,0.14) 0%, rgba(28,24,20,0.97) 60%), rgba(28,24,20,0.97)',
          border: '1px solid rgba(255, 141, 92, 0.2)',
          borderRadius: 24,
          padding: '32px 26px 26px',
          // Drop shadow handled by native macOS (set_has_shadow(true) in
          // show_onboarding_window). CSS provides the warm inner glow only.
          boxShadow: '0 0 40px rgba(255,100,40,0.07)',
          position: 'relative',
          overflow: 'hidden',
        }}
      >
        {/* Top edge highlight */}
        <div
          style={{
            position: 'absolute',
            top: 0,
            left: 0,
            right: 0,
            height: 1,
            background:
              'linear-gradient(90deg, transparent, rgba(255,141,92,0.4), transparent)',
          }}
        />

        {/* Logo mark + title, drag region so the user can reposition the
            onboarding window when it overlaps System Settings. */}
        <div
          data-tauri-drag-region
          style={{ textAlign: 'center', marginBottom: 18, cursor: 'grab' }}
        >
          <img
            src={thukiLogo}
            width={64}
            height={64}
            alt="Thuki"
            style={{
              objectFit: 'contain',
              pointerEvents: 'none',
              display: 'block',
              margin: '0 auto',
            }}
          />
        </div>

        {/* Title */}
        <h1
          style={{
            textAlign: 'center',
            fontSize: 22,
            fontWeight: 700,
            color: '#f0f0f2',
            letterSpacing: '-0.4px',
            lineHeight: 1.2,
            margin: '0 0 20px',
          }}
        >
          {"Let's get Thuki set up"}
        </h1>

        {/* Steps */}
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 10,
            marginBottom: 20,
          }}
        >
          {/* Step 1: Accessibility */}
          <StepCard active={!accessibilityGranted} done={accessibilityGranted}>
            <div
              style={{
                width: 36,
                height: 36,
                borderRadius: 10,
                flexShrink: 0,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                background: accessibilityGranted
                  ? 'rgba(34,197,94,0.12)'
                  : 'rgba(255,141,92,0.12)',
                border: `1px solid ${accessibilityGranted ? 'rgba(34,197,94,0.2)' : 'rgba(255,141,92,0.25)'}`,
              }}
            >
              {accessibilityGranted ? <CheckIcon /> : <KeyboardIcon />}
            </div>
            <div style={{ flex: 1 }}>
              <div
                style={{
                  fontSize: 14,
                  fontWeight: 600,
                  color: '#f0f0f2',
                  marginBottom: 2,
                }}
              >
                Accessibility
              </div>
              <div style={{ fontSize: 12, color: '#6b6660', lineHeight: 1.5 }}>
                Lets Thuki respond to activator key (<KeyChip label="⌃" />
                <KeyChip label="⌃" />)
              </div>
            </div>
            {accessibilityGranted && (
              <div style={{ flexShrink: 0 }}>
                <Badge color="green">Granted</Badge>
              </div>
            )}
          </StepCard>

          {/* Step 2: Screen Recording */}
          <StepCard active={accessibilityGranted} done={screenGranted}>
            <div
              style={{
                width: 36,
                height: 36,
                borderRadius: 10,
                flexShrink: 0,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                background: accessibilityGranted
                  ? 'rgba(255,141,92,0.12)'
                  : 'rgba(255,255,255,0.04)',
                border: `1px solid ${accessibilityGranted ? 'rgba(255,141,92,0.25)' : 'rgba(255,255,255,0.06)'}`,
              }}
            >
              <ScreenIcon active={accessibilityGranted} />
            </div>
            <div style={{ flex: 1 }}>
              <div
                style={{
                  fontSize: 14,
                  fontWeight: 600,
                  color: accessibilityGranted ? '#f0f0f2' : '#4a4a4e',
                  marginBottom: 2,
                }}
              >
                Screen Recording
              </div>
              <div style={{ fontSize: 12, color: '#6b6660', lineHeight: 1.35 }}>
                Needed for /screen to capture your entire screen
              </div>
            </div>
          </StepCard>
        </div>

        {/* Step 1 CTA: Grant Accessibility */}
        {!accessibilityGranted && (
          <CTAButton
            onClick={handleGrantAccessibility}
            disabled={isAxRequesting}
            aria-label={
              isAxRequesting ? 'Checking...' : 'Grant Accessibility Access'
            }
            loading={isAxRequesting}
          >
            {isAxRequesting ? 'Checking...' : 'Grant Accessibility Access'}
          </CTAButton>
        )}

        {/* Step 2 CTAs: Open Settings (with polling) + Quit & Reopen */}
        {accessibilityGranted && (
          <>
            {!screenGranted && (
              <CTAButton
                onClick={
                  isScreenPolling ? undefined : handleOpenScreenRecording
                }
                disabled={isScreenPolling}
                aria-label={
                  isScreenPolling
                    ? 'Checking...'
                    : 'Open Screen Recording Settings'
                }
                loading={isScreenPolling}
              >
                {isScreenPolling
                  ? 'Checking...'
                  : 'Open Screen Recording Settings'}
              </CTAButton>
            )}
            {screenGranted && (
              <>
                <CTAButton
                  onClick={handleQuitAndRelaunch}
                  aria-label="Quit and Reopen Thuki"
                >
                  Quit & Reopen Thuki
                </CTAButton>
                <p
                  style={{
                    textAlign: 'center',
                    fontSize: 11,
                    color: 'rgba(107,102,96,0.8)',
                    lineHeight: 1.4,
                    margin: 0,
                  }}
                >
                  macOS requires a restart for Screen Recording to take effect
                </p>
              </>
            )}
          </>
        )}
      </motion.div>
    </div>
  );
}

// ─── Sub-components ─────────────────────────────────────────────────────────

interface CTAButtonProps {
  onClick?: React.MouseEventHandler<HTMLButtonElement>;
  disabled?: boolean;
  'aria-label'?: string;
  loading?: boolean;
  children: React.ReactNode;
}

/** Primary action button with a subtle lift-and-brighten hover effect. */
function CTAButton({
  onClick,
  disabled,
  'aria-label': ariaLabel,
  loading,
  children,
}: CTAButtonProps) {
  const [hovered, setHovered] = useState(false);

  const isDisabled = disabled || loading;

  return (
    <button
      onClick={onClick}
      disabled={isDisabled}
      aria-label={ariaLabel}
      onMouseEnter={() => !isDisabled && setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 8,
        width: '100%',
        padding: '13px',
        background: isDisabled
          ? 'rgba(255,141,92,0.4)'
          : 'linear-gradient(135deg, #ff8d5c 0%, #d45a1e 100%)',
        color: 'white',
        fontSize: 14,
        fontWeight: 600,
        border: 'none',
        borderRadius: 14,
        cursor: isDisabled ? 'not-allowed' : 'pointer',
        letterSpacing: '-0.1px',
        marginBottom: 10,
        opacity: isDisabled ? 0.7 : 1,
        boxShadow: isDisabled
          ? 'none'
          : '0 4px 24px rgba(255,100,40,0.35), 0 1px 0 rgba(255,255,255,0.12) inset',
        filter: hovered && !isDisabled ? 'brightness(1.1)' : 'none',
        transition: 'filter 0.15s ease',
      }}
    >
      {loading && <Spinner />}
      {children}
    </button>
  );
}
