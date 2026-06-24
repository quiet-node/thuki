/**
 * Onboarding step shown once to upgraders coming from a pre-built-in-engine
 * version. It announces that Thuki now ships its own inference engine and lets
 * the user either adopt it ("Try Built-in Engine") or stay on their existing
 * Ollama setup ("Keep using Ollama"). Both choices are non-destructive: the
 * grandfathered Ollama provider is left intact until the user explicitly
 * switches.
 *
 * Reached only when the backend routes the `builtin_announcement` stage (see
 * `should_show_builtin_announcement` in `onboarding.rs`); brand-new installs,
 * which default to the built-in provider, never see it.
 *
 * Both branches hand off to the backend `advance_past_builtin_announcement`
 * command, which latches the one-time flag and advances to `model_check`. The
 * provider switch for the built-in branch reuses the shared `set_active_provider`
 * command, so no provider/unload logic is duplicated here.
 */

import { useCallback, useRef, useState } from 'react';
import { motion } from 'framer-motion';
import type React from 'react';
import { invoke } from '@tauri-apps/api/core';
import thukiLogo from '../../../src-tauri/icons/128x128.png';
import { useFitOnboardingWindow } from '../../hooks/useFitOnboardingWindow';

/** Built-in provider id, mirrored from the backend `PROVIDER_ID_BUILTIN`. */
const PROVIDER_ID_BUILTIN = 'builtin';

/**
 * Destination for the "Learn more" link. Points at the product site today;
 * repoint to the v0.15 built-in-engine blog post once it is published.
 */
const LEARN_MORE_URL = 'https://www.thuki.app';

/**
 * GitHub release page for the version that introduced the built-in engine.
 * Linked from the subtitle; the tag is not cut yet, so the page 404s until the
 * v0.15.0 release is published.
 */
const RELEASE_TAG_URL =
  'https://github.com/quiet-node/thuki/releases/tag/v0.15.0';

/** Hugging Face home, opened from the "Total AI model freedom" point. */
const HUGGING_FACE_URL = 'https://huggingface.co';

export function BuiltinAnnouncementStep() {
  const cardRef = useRef<HTMLDivElement>(null);
  // The onboarding window is transparent; fit it to the card so the empty area
  // never blocks clicks to the apps behind Thuki.
  useFitOnboardingWindow(cardRef, null);

  const handleTryBuiltin = useCallback(async () => {
    try {
      await invoke('set_active_provider', { providerId: PROVIDER_ID_BUILTIN });
    } catch {
      // Switch failed (e.g. config write error): stay on the announcement
      // rather than advancing into a picker for a provider we did not select.
      return;
    }
    await invoke('advance_past_builtin_announcement');
  }, []);

  const handleKeepOllama = useCallback(async () => {
    await invoke('advance_past_builtin_announcement');
  }, []);

  const handleLearnMore = useCallback(() => {
    void invoke('open_url', { url: LEARN_MORE_URL });
  }, []);

  const handleViewRelease = useCallback(() => {
    void invoke('open_url', { url: RELEASE_TAG_URL });
  }, []);

  const handleHuggingFace = useCallback(() => {
    void invoke('open_url', { url: HUGGING_FACE_URL });
  }, []);

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
          width: 450,
          // Flex child of a centering wrapper: never shrink to the window width
          // during a mid-resize measure (see the picker card in ModelCheckStep).
          flexShrink: 0,
          background:
            'radial-gradient(ellipse 80% 55% at 50% 0%, rgba(255,141,92,0.14) 0%, rgba(28,24,20,0.97) 60%), rgba(28,24,20,0.97)',
          border: '1px solid rgba(255, 141, 92, 0.2)',
          borderRadius: 24,
          padding: '28px 26px 22px',
          boxShadow: '0 0 40px rgba(255,100,40,0.07)',
          position: 'relative',
          overflow: 'hidden',
        }}
      >
        {/* Top edge highlight, identical to the other onboarding steps. */}
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

        {/* Drag handle: the logo area moves the window, matching the other
            onboarding steps. The image itself ignores pointer events so the
            drag region receives the gesture. */}
        <div
          data-tauri-drag-region
          style={{ textAlign: 'center', cursor: 'grab' }}
        >
          <img
            src={thukiLogo}
            width={46}
            height={46}
            alt="Thuki"
            style={{
              objectFit: 'contain',
              pointerEvents: 'none',
              display: 'block',
              margin: '0 auto 13px',
            }}
          />
        </div>

        <div style={{ textAlign: 'center' }}>
          <span
            style={{
              display: 'inline-block',
              fontSize: 10,
              fontWeight: 700,
              letterSpacing: '0.5px',
              color: '#ff8d5c',
              background: 'rgba(255,141,92,0.1)',
              border: '1px solid rgba(255,141,92,0.22)',
              borderRadius: 20,
              padding: '2px 9px',
              marginBottom: 13,
            }}
          >
            NEW
          </span>
        </div>

        <h1
          style={{
            textAlign: 'center',
            fontSize: 23,
            fontWeight: 700,
            color: '#f0f0f2',
            letterSpacing: '-0.4px',
            lineHeight: 1.2,
            margin: '0 0 8px',
          }}
        >
          Local AI, now built in
        </h1>
        <p
          style={{
            textAlign: 'center',
            fontSize: 12.5,
            color: 'rgba(255,255,255,0.55)',
            lineHeight: 1.5,
            margin: '0 0 20px',
            whiteSpace: 'nowrap',
          }}
        >
          Since{' '}
          <TextLink
            onClick={handleViewRelease}
            ariaLabel="View the v0.15 release on GitHub"
            color="rgba(255,141,92,0.85)"
            hoverColor="#ff8d5c"
            style={{
              display: 'inline',
              padding: 0,
              fontSize: 'inherit',
              fontWeight: 600,
            }}
          >
            v0.15
          </TextLink>
          , Thuki ships its own inference engine.
        </p>

        <div style={{ display: 'flex', flexDirection: 'column' }}>
          <Point
            icon={<BoltIcon />}
            title="One app, nothing else to manage"
            desc="No more background Ollama, Thuki runs models itself."
          />
          <Point
            icon={<LayersIcon />}
            title="Total AI model freedom"
            desc={
              <>
                Any model, any quantization on{' '}
                <TextLink
                  onClick={handleHuggingFace}
                  ariaLabel="Open Hugging Face"
                  color="rgba(255,141,92,0.85)"
                  hoverColor="#ff8d5c"
                  style={{
                    display: 'inline',
                    padding: 0,
                    fontSize: 'inherit',
                    fontWeight: 600,
                  }}
                >
                  Hugging Face
                </TextLink>{' '}
                your Mac can handle. Find and download them right in the app, no
                terminal needed.
              </>
            }
          />
          <Point
            icon={<ShieldIcon />}
            title="Private, exactly like before"
            desc="Every model runs locally. Nothing leaves your Mac."
            last
          />
        </div>

        <div
          style={{
            height: 1,
            background: 'rgba(255,255,255,0.05)',
            margin: '6px 0 14px',
          }}
        />

        <button
          onClick={() => void handleTryBuiltin()}
          aria-label="Try Built-in Engine"
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
            fontFamily: 'inherit',
          }}
        >
          Try Built-in Engine
        </button>

        <TextLink
          onClick={() => void handleKeepOllama()}
          ariaLabel="Keep using Ollama"
          color="rgba(255,255,255,0.4)"
          hoverColor="rgba(255,255,255,0.7)"
          style={{
            display: 'block',
            width: '100%',
            textAlign: 'center',
            marginTop: 11,
            fontSize: 12,
            fontWeight: 500,
          }}
        >
          Keep using Ollama
        </TextLink>

        <p
          style={{
            textAlign: 'center',
            fontSize: 11,
            color: 'rgba(255,255,255,0.3)',
            lineHeight: 1.5,
            margin: '22px 0 0',
          }}
        >
          Either way, you can switch engines anytime in Settings.
        </p>

        <p
          style={{
            textAlign: 'center',
            fontSize: 10.5,
            color: 'rgba(255,255,255,0.22)',
            marginTop: 4,
            lineHeight: 1.6,
          }}
        >
          Added in v0.15 &middot;{' '}
          <TextLink
            onClick={handleLearnMore}
            ariaLabel="Learn more about the built-in engine"
            color="rgba(255,141,92,0.55)"
            hoverColor="#ff8d5c"
            style={{ display: 'inline', padding: 0, fontSize: 'inherit' }}
          >
            Learn more ↗
          </TextLink>
        </p>
      </motion.div>
    </div>
  );
}

// ─── Sub-components ──────────────────────────────────────────────────────────

interface PointProps {
  icon: React.ReactNode;
  title: string;
  desc: React.ReactNode;
  last?: boolean;
}

/** A single benefit row: orange glyph, bold title, muted description. */
function Point({ icon, title, desc, last = false }: PointProps) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'flex-start',
        gap: 13,
        padding: '12px 0',
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
          color: 'rgba(255,141,92,0.7)',
        }}
      >
        {icon}
      </div>
      <div>
        <div
          style={{
            fontSize: 13,
            fontWeight: 600,
            color: 'rgba(240,240,242,0.92)',
            marginBottom: 2,
            letterSpacing: '-0.1px',
            lineHeight: 1.4,
          }}
        >
          {title}
        </div>
        <div
          style={{
            fontSize: 11.5,
            color: 'rgba(255,255,255,0.42)',
            lineHeight: 1.5,
          }}
        >
          {desc}
        </div>
      </div>
    </div>
  );
}

interface TextLinkProps {
  onClick: () => void;
  ariaLabel: string;
  color: string;
  hoverColor: string;
  style: React.CSSProperties;
  children: React.ReactNode;
}

/**
 * Text-styled button that lifts to `hoverColor` on hover. Shared by the
 * "Keep using Ollama" secondary action and the footer "Learn more" link so the
 * hover treatment lives in one place.
 */
function TextLink({
  onClick,
  ariaLabel,
  color,
  hoverColor,
  style,
  children,
}: TextLinkProps) {
  const [hover, setHover] = useState(false);
  return (
    <button
      type="button"
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-label={ariaLabel}
      style={{
        background: 'none',
        border: 'none',
        cursor: 'pointer',
        fontFamily: 'inherit',
        fontWeight: 500,
        color: hover ? hoverColor : color,
        transition: 'color 160ms ease',
        ...style,
      }}
    >
      {children}
    </button>
  );
}

// ─── Glyphs ──────────────────────────────────────────────────────────────────

function BoltIcon() {
  return (
    <svg
      width="19"
      height="19"
      viewBox="0 0 20 20"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M11 2L4 11h5l-1 7 7-9h-5l1-7z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function LayersIcon() {
  return (
    <svg
      width="19"
      height="19"
      viewBox="0 0 20 20"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M10 3l7 3.5-7 3.5-7-3.5L10 3z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path
        d="M3 10l7 3.5L17 10M3 13.5L10 17l7-3.5"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
        opacity="0.55"
      />
    </svg>
  );
}

function ShieldIcon() {
  return (
    <svg
      width="19"
      height="19"
      viewBox="0 0 20 20"
      fill="none"
      aria-hidden="true"
    >
      <path
        d="M10 2l6 2.2v4.3c0 4-2.6 6.6-6 8.2-3.4-1.6-6-4.2-6-8.2V4.2L10 2z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </svg>
  );
}
