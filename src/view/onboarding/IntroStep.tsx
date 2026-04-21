import { motion } from 'framer-motion';
import { invoke } from '@tauri-apps/api/core';
import thukiLogo from '../../../src-tauri/icons/128x128.png';

interface Props {
  onComplete: () => void;
}

export function IntroStep({ onComplete }: Props) {
  const handleGetStarted = async () => {
    await invoke('finish_onboarding');
    onComplete();
  };

  return (
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
          boxShadow: '0 0 40px rgba(255,100,40,0.07)',
          position: 'relative',
        }}
      >
        {/* Logo */}
        <img
          src={thukiLogo}
          width={44}
          height={44}
          alt="Thuki"
          style={{
            objectFit: 'contain',
            display: 'block',
            margin: '0 auto 16px',
            pointerEvents: 'none',
          }}
        />

        {/* Header */}
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
            Before you dive in
          </h1>
          <p
            style={{
              fontSize: 13,
              color: 'rgba(255,255,255,0.3)',
              lineHeight: 1.6,
              margin: 0,
            }}
          >
            {"You'll get the hang of it quickly."}
          </p>
        </div>

        {/* Facts */}
        <div style={{ display: 'flex', flexDirection: 'column' }}>
          <Fact
            icon={<KeyboardIcon />}
            title={
              <>
                <span>Double-tap</span> <KeyChip>⌃</KeyChip>{' '}
                <span>to summon</span>
              </>
            }
            desc="Press Control twice from any app, any time"
          />
          <Fact
            icon={<SelectionIcon />}
            title={
              <>
                <span>Select text, then double-tap</span> <KeyChip>⌃</KeyChip>
              </>
            }
            desc="It opens with your selection already quoted as context"
          />
          <Fact
            icon={<ImageIcon />}
            title="Drop in any image"
            desc="Paste, drag, or clip a screenshot straight into the conversation."
          />
          <Fact
            icon={<CommandIcon />}
            title={
              <>
                <span>Type</span> <MonoChip>/</MonoChip>{' '}
                <span>for commands</span>
              </>
            }
            desc="Open the slash menu for built-in tools and writing shortcuts right from the ask bar."
          />
          <Fact
            icon={<FloatIcon />}
            title="Floats above everything"
            desc="No app switching, no broken flow. Summon it, use it, toss the chat and get straight back to work."
            last
          />
        </div>

        {/* Divider */}
        <div
          style={{
            height: 1,
            background: 'rgba(255,255,255,0.05)',
            margin: '16px 0',
          }}
        />

        {/* CTA */}
        <button
          onClick={() => void handleGetStarted()}
          aria-label="Get Started"
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
            cursor: 'pointer',
            letterSpacing: '-0.1px',
            boxShadow: '0 4px 20px rgba(255,100,40,0.28)',
            textAlign: 'center',
          }}
        >
          Get Started
        </button>

        {/* Footer */}
        <p
          style={{
            textAlign: 'center',
            fontSize: 11,
            color: 'rgba(255,255,255,0.18)',
            marginTop: 14,
            lineHeight: 1.5,
          }}
        >
          Private by default &middot; All inference runs on your machine
        </p>
      </motion.div>
    </div>
  );
}

// ─── Sub-components ──────────────────────────────────────────────────────────

interface FactProps {
  icon: React.ReactNode;
  title: React.ReactNode;
  desc: string;
  last?: boolean;
}

function Fact({ icon, title, desc, last = false }: FactProps) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'flex-start',
        gap: 14,
        padding: '11px 0',
        borderBottom: last ? 'none' : '1px solid rgba(255,255,255,0.05)',
      }}
    >
      <div
        style={{
          width: 30,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexShrink: 0,
          paddingTop: 1,
          color: 'rgba(255,141,92,0.65)',
        }}
      >
        {icon}
      </div>
      <div>
        <div
          style={{
            fontSize: 13,
            fontWeight: 600,
            color: 'rgba(240,240,242,0.9)',
            marginBottom: 2,
            display: 'flex',
            alignItems: 'center',
            gap: 4,
            flexWrap: 'wrap',
            lineHeight: 1.4,
          }}
        >
          {title}
        </div>
        <div
          style={{
            fontSize: 11,
            color: 'rgba(255,255,255,0.28)',
            lineHeight: 1.5,
          }}
        >
          {desc}
        </div>
      </div>
    </div>
  );
}

function KeyChip({ children }: { children: React.ReactNode }) {
  return (
    <span
      style={{
        display: 'inline-block',
        padding: '1px 6px',
        background: 'rgba(255,255,255,0.06)',
        border: '1px solid rgba(255,255,255,0.12)',
        borderBottom: '2px solid rgba(255,255,255,0.08)',
        borderRadius: 4,
        fontSize: 11,
        color: 'rgba(255,255,255,0.55)',
        lineHeight: 1.5,
        verticalAlign: 'middle',
      }}
    >
      {children}
    </span>
  );
}

function MonoChip({ children }: { children: React.ReactNode }) {
  return (
    <span
      style={{
        display: 'inline-block',
        padding: '1px 6px',
        background: 'rgba(255,141,92,0.08)',
        border: '1px solid rgba(255,141,92,0.15)',
        borderRadius: 4,
        fontSize: 11,
        color: 'rgba(255,141,92,0.75)',
        fontFamily: "'SF Mono', 'Fira Mono', monospace",
        lineHeight: 1.5,
        verticalAlign: 'middle',
      }}
    >
      {children}
    </span>
  );
}

function KeyboardIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect
        x="2"
        y="4"
        width="14"
        height="10"
        rx="2"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path
        d="M5 8h1M8 8h1M11 8h1M5 11h8"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

function SelectionIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect
        x="2"
        y="6"
        width="10"
        height="6"
        rx="1.5"
        fill="currentColor"
        opacity="0.18"
      />
      <rect
        x="2"
        y="6"
        width="10"
        height="6"
        rx="1.5"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path
        d="M14 4v10"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
      <path
        d="M12 4h4M12 14h4"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
      />
    </svg>
  );
}

function ImageIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect
        x="2"
        y="4"
        width="14"
        height="10"
        rx="2"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path
        d="M2 11l3.5-3.5 3 3 2.5-2.5 4 4"
        stroke="currentColor"
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity="0.6"
      />
      <circle cx="6" cy="7.5" r="1.2" fill="currentColor" opacity="0.5" />
    </svg>
  );
}

function CommandIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect
        x="2"
        y="4"
        width="14"
        height="10"
        rx="2"
        stroke="currentColor"
        strokeWidth="1.5"
      />
      <path
        d="M5.5 7.25l2 2-2 2"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M10.75 6.25l-2 6"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  );
}

function FloatIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <rect
        x="2"
        y="7"
        width="12"
        height="8"
        rx="2"
        stroke="currentColor"
        strokeWidth="1.5"
        opacity="0.4"
      />
      <rect
        x="4"
        y="3"
        width="12"
        height="8"
        rx="2"
        fill="rgba(22,18,15,0.98)"
        stroke="currentColor"
        strokeWidth="1.5"
      />
    </svg>
  );
}
