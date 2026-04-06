import { motion } from 'framer-motion';
import type React from 'react';
import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** How often to poll for permission grants after the user requests them. */
const POLL_INTERVAL_MS = 500;

type AccessibilityStatus = 'pending' | 'requesting' | 'granted';
type ScreenRecordingStatus = 'idle' | 'polling' | 'granted';

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
 * Visual direction: Warm Ambient — dark base with a warm orange radial glow.
 * The outer container is transparent so the rounded panel corners are visible
 * against the macOS desktop.
 */
export function OnboardingView() {
  const [accessibilityStatus, setAccessibilityStatus] =
    useState<AccessibilityStatus>('pending');
  const [screenRecordingStatus, setScreenRecordingStatus] =
    useState<ScreenRecordingStatus>('idle');
  const axPollRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const screenPollRef = useRef<ReturnType<typeof setInterval> | null>(null);

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

  // On mount: check whether Accessibility is already granted so we can skip
  // step 1 and show step 2 immediately.
  useEffect(() => {
    void invoke<boolean>('check_accessibility_permission').then((granted) => {
      if (granted) {
        setAccessibilityStatus('granted');
      }
    });
    return () => {
      stopAxPolling();
      stopScreenPolling();
    };
  }, [stopAxPolling, stopScreenPolling]);

  const handleGrantAccessibility = useCallback(async () => {
    setAccessibilityStatus('requesting');
    await invoke('open_accessibility_settings');
    axPollRef.current = setInterval(async () => {
      const granted = await invoke<boolean>('check_accessibility_permission');
      if (granted) {
        stopAxPolling();
        setAccessibilityStatus('granted');
      }
    }, POLL_INTERVAL_MS);
  }, [stopAxPolling]);

  const handleOpenScreenRecording = useCallback(async () => {
    // Register Thuki in TCC (adds it to the Screen Recording list) then open
    // System Settings directly so the user can toggle it on without hunting.
    // The registration call may briefly show a macOS system prompt on first use.
    await invoke('request_screen_recording_access');
    await invoke('open_screen_recording_settings');
    setScreenRecordingStatus('polling');
    screenPollRef.current = setInterval(async () => {
      const granted = await invoke<boolean>(
        'check_screen_recording_tcc_granted',
      );
      if (granted) {
        stopScreenPolling();
        setScreenRecordingStatus('granted');
      }
    }, POLL_INTERVAL_MS);
  }, [stopScreenPolling]);

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
        fontFamily: 'Inter, -apple-system, BlinkMacSystemFont, sans-serif',
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

        {/* Logo mark + title — drag region so the user can reposition the
            onboarding window when it overlaps System Settings. */}
        <div
          data-tauri-drag-region
          style={{ textAlign: 'center', marginBottom: 18, cursor: 'grab' }}
        >
          <div
            style={{
              display: 'inline-flex',
              width: 48,
              height: 48,
              borderRadius: 14,
              background:
                'linear-gradient(135deg, rgba(255,141,92,0.2), rgba(224,107,48,0.1))',
              border: '1px solid rgba(255,141,92,0.25)',
              alignItems: 'center',
              justifyContent: 'center',
              boxShadow: '0 0 20px rgba(255,141,92,0.15)',
              pointerEvents: 'none',
            }}
          >
            <svg
              width="24"
              height="24"
              viewBox="0 0 24 24"
              fill="none"
              aria-hidden="true"
            >
              <circle cx="12" cy="12" r="4" fill="#ff8d5c" />
              <circle
                cx="12"
                cy="12"
                r="8"
                stroke="#ff8d5c"
                strokeWidth="1.2"
                strokeDasharray="2 3"
                opacity="0.5"
              />
            </svg>
          </div>
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
            margin: '0 0 5px',
          }}
        >
          {"Let's get Thuki set up"}
        </h1>
        <p
          style={{
            textAlign: 'center',
            fontSize: 13,
            color: '#6b6660',
            margin: '0 0 26px',
          }}
        >
          Two quick steps, then your AI is ready
        </p>

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
              <div style={{ fontSize: 12, color: '#6b6660', lineHeight: 1.35 }}>
                Lets Thuki respond to your Control key
              </div>
            </div>
            <div style={{ flexShrink: 0 }}>
              {accessibilityGranted ? (
                <Badge color="green">Granted</Badge>
              ) : (
                <Badge color="orange">Step 1</Badge>
              )}
            </div>
          </StepCard>

          {/* Step 2: Screen Recording */}
          <StepCard active={accessibilityGranted} done={false}>
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
                Needed for the /screen command
              </div>
            </div>
            <div style={{ flexShrink: 0 }}>
              <Badge color={accessibilityGranted ? 'orange' : 'muted'}>
                Step 2
              </Badge>
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

interface StepCardProps {
  active: boolean;
  done: boolean;
  children: React.ReactNode;
}

function StepCard({ active, done, children }: StepCardProps) {
  const borderColor = done
    ? 'rgba(34,197,94,0.2)'
    : active
      ? 'rgba(255,141,92,0.4)'
      : 'rgba(255,255,255,0.06)';

  const background = done
    ? 'rgba(34,197,94,0.05)'
    : active
      ? 'rgba(255,141,92,0.07)'
      : 'rgba(255,255,255,0.03)';

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '14px 16px',
        borderRadius: 16,
        border: `1px solid ${borderColor}`,
        background,
        boxShadow:
          active && !done
            ? '0 0 20px rgba(255,141,92,0.08), inset 0 1px 0 rgba(255,141,92,0.1)'
            : 'none',
      }}
    >
      {children}
    </div>
  );
}

interface BadgeProps {
  color: 'green' | 'orange' | 'muted';
  children: React.ReactNode;
}

function Badge({ color, children }: BadgeProps) {
  const styles: Record<string, React.CSSProperties> = {
    green: {
      color: '#22c55e',
      background: 'rgba(34,197,94,0.1)',
      border: '1px solid rgba(34,197,94,0.2)',
    },
    orange: {
      color: '#ff8d5c',
      background: 'rgba(255,141,92,0.1)',
      border: '1px solid rgba(255,141,92,0.2)',
    },
    muted: {
      color: '#4a4a4e',
      background: 'rgba(255,255,255,0.04)',
      border: '1px solid rgba(255,255,255,0.06)',
    },
  };

  return (
    <span
      style={{
        fontSize: 11,
        fontWeight: 600,
        padding: '3px 9px',
        borderRadius: 20,
        ...styles[color],
      }}
    >
      {children}
    </span>
  );
}
