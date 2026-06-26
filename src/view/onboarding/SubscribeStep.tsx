import { useRef, useState } from 'react';
import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import thukiLogo from '../../../src-tauri/icons/128x128.png';
import { useFitOnboardingWindow } from '../../hooks/useFitOnboardingWindow';
import { InlineLink } from '../../components/InlineLink';
import { isValidEmail } from '../../utils/email';
import {
  DownloadStatusStrip,
  type DownloadStripStatus,
} from '../../components/DownloadStatusStrip';

/**
 * Founder's X profile, opened in the user's browser from the inline "Logan"
 * link in the note. Routed through the `open_url` command (never an anchor
 * navigating the webview) so the onboarding window is never repurposed as a
 * browser. Matches the social links in the About tab.
 */
const X_PROFILE_URL = 'https://x.com/quiet_node';

interface RoadmapItem {
  icon: React.ReactNode;
  title: string;
  desc: string;
}

const ROADMAP: ReadonlyArray<RoadmapItem> = [
  {
    icon: <LinkIcon />,
    title: 'Connect your tools',
    desc: 'Gmail, Slack, Discord, Calendar, and more.',
  },
  {
    icon: <MicIcon />,
    title: 'Type with your voice',
    desc: 'Press a key, speak, and get clean text in any app.',
  },
  {
    icon: <WaveformIcon />,
    title: 'Notes from any meeting',
    desc: 'Live transcripts and summaries of any meeting.',
  },
  {
    icon: <ZapIcon />,
    title: 'Automate the routine',
    desc: 'Teach Thuki multi-step tasks and run them on a word.',
  },
];

interface Props {
  /**
   * Advance out of the roadmap screen to the next onboarding step (the
   * "You're all set" tips card). Called both when the user subscribes with a
   * valid email and when they skip; this screen never finishes onboarding
   * itself, it only hands off.
   */
  onContinue: () => void;
  /**
   * Ambient background-download status, rendered at the card base while a
   * built-in model finishes downloading during onboarding. Mirrors the strip
   * on the tips card so progress stays visible across both screens. `null` /
   * omitted renders nothing.
   */
  downloadStatus?: DownloadStripStatus | null;
}

/**
 * Optional roadmap-and-email onboarding step.
 *
 * Shown once, right before the final tips card. It previews what is coming to
 * Thuki and invites the user to leave their email so the founder can learn how
 * they use it and shape the roadmap. The ask is never required: "Maybe later"
 * advances just like a successful subscribe.
 *
 * The email is only submitted on an explicit click with a valid address, never
 * automatically, which keeps this consistent with the app's "no silent
 * phone-home" posture. `handleSubscribe` validates, hands the address to the
 * `subscribe_email` backend command, and advances on success; a failed send
 * shows a gentle inline notice without trapping the user, so "Maybe later"
 * always remains a way out.
 */
export function SubscribeStep({ onContinue, downloadStatus }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [email, setEmail] = useState('');
  const [invalid, setInvalid] = useState(false);
  const [focused, setFocused] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [sendFailed, setSendFailed] = useState(false);
  // Re-fit the transparent onboarding window when the ambient download strip
  // appears or changes height so the card never leaves a click-blocking
  // margin. The inline error line is absorbed by the hook's ResizeObserver.
  useFitOnboardingWindow(cardRef, downloadStatus);

  const handleSubscribe = async () => {
    const trimmed = email.trim();
    if (!isValidEmail(trimmed)) {
      setInvalid(true);
      return;
    }
    setSubmitting(true);
    setSendFailed(false);
    try {
      // Sent only here, on an explicit click with a valid address. An
      // already-subscribed address resolves successfully, so re-onboarding is
      // never an error.
      await invoke('subscribe_email', { email: trimmed });
      // onContinue unmounts this screen, so there is no submitting state to
      // reset on the success path.
      onContinue();
    } catch {
      // Surface a gentle notice and let the user retry or skip; never block
      // the flow on a failed send.
      setSendFailed(true);
      setSubmitting(false);
    }
  };

  const handleEmailChange = (next: string) => {
    setEmail(next);
    // Clear any error as soon as the user edits, so neither the validation nor
    // the send-failure message lingers over input they are actively fixing.
    if (invalid) setInvalid(false);
    if (sendFailed) setSendFailed(false);
  };

  return (
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
        ref={cardRef}
        initial={{ opacity: 0, scale: 0.97, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ type: 'spring', stiffness: 300, damping: 28 }}
        style={{
          width: 420,
          flexShrink: 0,
          background:
            'radial-gradient(ellipse 80% 55% at 50% 0%, rgba(255,141,92,0.14) 0%, rgba(28,24,20,0.97) 60%), rgba(28,24,20,0.97)',
          border: '1px solid rgba(255, 141, 92, 0.2)',
          borderRadius: 24,
          padding: '30px 28px 24px',
          boxShadow: '0 0 40px rgba(255,100,40,0.07)',
          position: 'relative',
        }}
      >
        {/* Logo doubles as the window drag handle, matching the other
            onboarding steps. The image ignores pointer events so the drag
            region receives the gesture. */}
        <div
          data-tauri-drag-region
          style={{ textAlign: 'center', cursor: 'grab' }}
        >
          <img
            src={thukiLogo}
            width={44}
            height={44}
            alt="Thuki"
            style={{
              objectFit: 'contain',
              display: 'block',
              margin: '0 auto 14px',
              pointerEvents: 'none',
            }}
          />
        </div>

        {/* Header — title + subtitle styling and spacing mirror IntroStep so
            the onboarding screens stay visually consistent. */}
        <div style={{ textAlign: 'center', marginBottom: 20 }}>
          <h1
            style={{
              fontSize: 21,
              fontWeight: 700,
              color: '#f0f0f2',
              letterSpacing: '-0.5px',
              lineHeight: 1.25,
              margin: '0 0 6px',
            }}
          >
            Where Thuki is headed
          </h1>
          <p
            style={{
              fontSize: 13,
              color: 'rgba(255,255,255,0.3)',
              lineHeight: 1.6,
              margin: 0,
            }}
          >
            A preview of what's coming.
          </p>
        </div>

        {/* Roadmap timeline */}
        <div style={{ position: 'relative', margin: '2px 2px 14px' }}>
          <div
            style={{
              position: 'absolute',
              left: 14,
              top: 20,
              bottom: 20,
              width: 2,
              background:
                'linear-gradient(rgba(255,141,92,0.4), rgba(255,141,92,0.06))',
            }}
          />
          {ROADMAP.map((item) => (
            <RoadmapRow key={item.title} item={item} />
          ))}
        </div>

        {/* Free + local guarantee, capping the roadmap. It applies to the list
            specifically (voice and meeting notes are paid, cloud features
            elsewhere) and is the only product-local line on this screen, so it
            repeats neither the subtitle nor the email trust line below. */}
        <p
          style={{
            textAlign: 'center',
            fontSize: 11.5,
            color: 'rgba(255,255,255,0.5)',
            lineHeight: 1.5,
            margin: '0 0 12px',
          }}
        >
          All free. All local. Nothing ever leaves your Mac.
        </p>

        {/* Divider */}
        <div
          style={{
            height: 1,
            background: 'rgba(255,255,255,0.06)',
            margin: '4px 0 14px',
          }}
        />

        {/* Founder note */}
        <p
          style={{
            fontSize: 12.5,
            fontStyle: 'italic',
            color: 'rgba(255,255,255,0.78)',
            lineHeight: 1.68,
            textAlign: 'left',
            margin: '0 0 16px',
          }}
        >
          <Quote />
          Hey there, I'm{' '}
          <InlineLink
            url={X_PROFILE_URL}
            ariaLabel="Open Logan's profile on X"
            style={{ fontStyle: 'normal', fontWeight: 600 }}
          >
            Logan
          </InlineLink>
          , founder of Thuki. I'd love to learn how you actually use it and hear
          your ideas, so I can shape these upcoming features to genuinely help
          you. Leave your email and I'll personally reach out, I'd love to talk!
          <Quote closing />
        </p>

        {/* Email field */}
        <input
          type="email"
          value={email}
          onChange={(e) => handleEmailChange(e.target.value)}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          placeholder="you@example.com"
          aria-label="Email address"
          aria-invalid={invalid}
          aria-describedby={invalid ? 'subscribe-email-error' : undefined}
          style={{
            width: '100%',
            background: 'rgba(0,0,0,0.28)',
            // The default focus ring picks up the user's macOS accent colour;
            // suppress it (`outline: none`) and signal focus with a calm accent
            // border instead, keeping a visible focus cue without the loud ring.
            border: `1px solid ${
              invalid
                ? 'rgba(255,138,128,0.6)'
                : focused
                  ? 'rgba(255,141,92,0.5)'
                  : 'rgba(255,255,255,0.1)'
            }`,
            borderRadius: 11,
            padding: '11px 13px',
            color: '#f0f0f2',
            fontSize: 13,
            fontFamily: 'inherit',
            outline: 'none',
            marginBottom: invalid ? 7 : 10,
          }}
        />
        {invalid ? (
          <p
            id="subscribe-email-error"
            style={{
              fontSize: 11,
              color: '#ff8a80',
              lineHeight: 1.4,
              margin: '0 0 10px',
            }}
          >
            Enter a valid email address.
          </p>
        ) : null}

        {/* Primary action. The aria-label stays fixed while the visible label
            switches to a sending state, so the button keeps a stable
            accessible name across the in-flight transition. */}
        <button
          onClick={() => void handleSubscribe()}
          disabled={submitting}
          aria-label="Help shape what's next for Thuki"
          style={{
            display: 'block',
            width: '100%',
            padding: '12px',
            background: 'linear-gradient(135deg, #ff8d5c 0%, #d45a1e 100%)',
            color: 'white',
            fontSize: 14,
            fontWeight: 600,
            border: 'none',
            borderRadius: 12,
            cursor: submitting ? 'default' : 'pointer',
            opacity: submitting ? 0.7 : 1,
            letterSpacing: '-0.1px',
            boxShadow: '0 4px 20px rgba(255,100,40,0.28)',
            textAlign: 'center',
          }}
        >
          {submitting ? 'Sending…' : "Help shape what's next for Thuki"}
        </button>

        {sendFailed ? (
          <p
            role="alert"
            style={{
              fontSize: 11,
              color: '#ff8a80',
              lineHeight: 1.45,
              textAlign: 'center',
              margin: '8px 0 0',
            }}
          >
            Couldn't send right now. Try again, or skip with "Maybe later".
          </p>
        ) : null}

        {/* Skip */}
        <button
          onClick={onContinue}
          aria-label="Maybe later"
          style={{
            display: 'block',
            width: '100%',
            marginTop: 8,
            padding: '9px',
            background: 'transparent',
            border: 'none',
            color: 'rgba(255,255,255,0.4)',
            fontSize: 12.5,
            fontWeight: 500,
            cursor: 'pointer',
          }}
        >
          Maybe later
        </button>

        {/* Trust line */}
        <p
          style={{
            textAlign: 'center',
            fontSize: 10.5,
            color: 'rgba(255,255,255,0.4)',
            marginTop: 13,
            lineHeight: 1.6,
          }}
        >
          No spam, no tracking, never shared or sold. Unsubscribe anytime.
        </p>

        {/* Ambient download strip, rendered at the card base so it reads as
            part of the screen, mirroring the tips card. The negative side
            margins pull it out to the card edges like IntroStep's strip. The
            "onboarding-roadmap" surface keeps the ready line visible but with a
            message that fits this screen, since "Get Started" lives on the tips
            card, not here. */}
        {downloadStatus ? (
          <div style={{ marginTop: 4, marginLeft: -16, marginRight: -16 }}>
            <DownloadStatusStrip
              status={downloadStatus}
              surface="onboarding-roadmap"
            />
          </div>
        ) : null}
      </motion.div>
    </div>
  );
}

// ─── Sub-components ──────────────────────────────────────────────────────────

function RoadmapRow({ item }: { item: RoadmapItem }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'flex-start',
        gap: 14,
        padding: '7px 0',
        position: 'relative',
      }}
    >
      <div
        style={{
          width: 28,
          height: 28,
          borderRadius: '50%',
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          background: '#1c1814',
          border: '1px solid rgba(255,141,92,0.35)',
          color: 'rgba(255,141,92,0.9)',
          position: 'relative',
          zIndex: 1,
        }}
      >
        {item.icon}
      </div>
      <div style={{ paddingTop: 2 }}>
        <div
          style={{
            fontSize: 13,
            fontWeight: 600,
            color: 'rgba(240,240,242,0.92)',
            lineHeight: 1.35,
          }}
        >
          {item.title}
        </div>
        <div
          style={{
            fontSize: 11,
            color: 'rgba(255,255,255,0.32)',
            lineHeight: 1.45,
            marginTop: 2,
          }}
        >
          {item.desc}
        </div>
      </div>
    </div>
  );
}

/**
 * A single serif quotation mark for the founder note. Rendered larger and warm
 * so the note reads as a pull-quote; hugs the first and last word inline. The
 * opening and closing marks sit at slightly different baselines, matching how
 * display quotes are set.
 */
function Quote({ closing = false }: { closing?: boolean }) {
  return (
    <span
      aria-hidden="true"
      style={{
        fontFamily: "'Georgia', 'Times New Roman', serif",
        fontStyle: 'normal',
        fontSize: 22,
        fontWeight: 700,
        color: 'rgba(255,141,92,0.7)',
        lineHeight: 0,
        verticalAlign: closing ? '-0.44em' : '-0.34em',
        marginLeft: closing ? 2 : 0,
        marginRight: closing ? 0 : 1,
      }}
    >
      {closing ? '”' : '“'}
    </span>
  );
}

function LinkIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
      <path
        d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function MicIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
      <rect
        x="9"
        y="2"
        width="6"
        height="12"
        rx="3"
        stroke="currentColor"
        strokeWidth="1.9"
      />
      <path
        d="M5 10a7 7 0 0 0 14 0"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
      />
      <line
        x1="12"
        y1="19"
        x2="12"
        y2="22"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
      />
    </svg>
  );
}

function WaveformIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
      <path
        d="M4 10v4M8 7v10M12 4v16M16 8v8M20 11v2"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
      />
    </svg>
  );
}

function ZapIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" fill="none">
      <polygon
        points="13 2 3 14 12 14 11 22 21 10 12 10 13 2"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
