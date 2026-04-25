/**
 * Onboarding step that gates the chat overlay on a working local Ollama
 * setup with at least one installed model.
 *
 * Mounts after PermissionsStep clears all macOS grants and before
 * IntroStep runs. Probes the daemon via the `check_model_setup` Tauri
 * command, then renders one of three states:
 *
 *   - Ollama unreachable: Step 1 is the active card with install /
 *     start affordances; Step 2 is the waiting card.
 *   - No models installed: Step 1 collapses to a green Connected
 *     badge; Step 2 is the active card with the recommended-model list.
 *   - Ready: never visible. The component fires `advance_past_model_check`
 *     and the parent OnboardingView replaces it with IntroStep before
 *     the next paint.
 *
 * Every state share the same screen, the same StepCard pattern, and the
 * same Re-check CTA, so the user is never surprised by a "wait, ANOTHER
 * last thing" tagline.
 */

import { motion } from 'framer-motion';
import type React from 'react';
import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import thukiLogo from '../../../src-tauri/icons/128x128.png';
import { StepCard, Badge } from './_shared';

/**
 * Wire-format payload returned by the `check_model_setup` Tauri command.
 *
 * Discriminated on `state` to match the Rust `ModelSetupState` enum
 * exactly. Frontend routes solely on the `state` string; the optional
 * `active_slug` and `installed` fields are present only on `ready`.
 */
type ModelSetupState =
  | { state: 'ollama_unreachable' }
  | { state: 'no_models_installed' }
  | { state: 'ready'; active_slug: string; installed: string[] };

/**
 * Recommended starter models surfaced in the Step 2 card.
 *
 * The list is intentionally short and curated: three options is enough
 * for new users to feel they have a real choice without forcing them
 * to research model trade-offs. Sourced from the design approved
 * 2026-04-25 in `~/.gstack/projects/.../approved.json`.
 */
const RECOMMENDED_MODELS: Array<{
  slug: string;
  description: string;
  recommended?: boolean;
}> = [
  {
    slug: 'gemma4:e2b',
    description: 'Lightweight all-rounder · 1.6 GB',
    recommended: true,
  },
  { slug: 'llama3:8b', description: 'Stronger reasoning · 4.7 GB' },
  { slug: 'qwen2.5:7b', description: 'Code-focused · 4.4 GB' },
];

/**
 * Builds the `ollama pull <slug>` command for a given model. Centralised
 * so the copy-button affordance and the (future) settings panel share
 * the same string and cannot drift.
 */
function buildPullCommand(slug: string): string {
  return `ollama pull ${slug}`;
}

/** Copies a string to the macOS clipboard, ignoring failures silently. */
async function copyToClipboard(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    // Clipboard write can fail on locked sessions or denied permissions;
    // there is no recovery and showing an error here would be more
    // confusing than silent. The terminal command remains visible for
    // the user to copy manually.
  }
}

/**
 * Renders the model-check onboarding gate.
 *
 * Probes Ollama once on mount and again on every Re-check click. No
 * background polling: the user is the trigger, which keeps idle CPU
 * and IPC traffic at zero between explicit interactions.
 *
 * Takes no props. Parent OnboardingView routes here when the persisted
 * stage is `model_check`. Stage advance to `intro` is owned by the
 * backend `advance_past_model_check` command, fired from inside this
 * component when the probe reports `Ready`. The backend re-emits the
 * onboarding event with the new stage so the parent re-routes without
 * a window flicker.
 */
export function ModelCheckStep() {
  const [setupState, setSetupState] = useState<ModelSetupState | null>(null);
  const [isRechecking, setIsRechecking] = useState(false);
  const mountedRef = useRef(true);

  /**
   * Probes Ollama via the backend command and either advances the
   * onboarding stage (Ready) or stores the gate state for rendering.
   *
   * Idempotent: safe to call repeatedly. The backend handles persisting
   * the resolved active slug; this hook only routes UI state.
   */
  const probe = useCallback(async () => {
    try {
      const next = await invoke<ModelSetupState>('check_model_setup');
      if (!mountedRef.current) return;
      if (next.state === 'ready') {
        // Fire-and-forget: the backend emits the onboarding event with
        // the new stage, which OnboardingView routes to IntroStep.
        await invoke('advance_past_model_check');
        return;
      }
      setSetupState(next);
    } catch {
      // Treat any IPC failure as Ollama unreachable so the user sees a
      // recovery path. The next Re-check click will retry.
      if (!mountedRef.current) return;
      setSetupState({ state: 'ollama_unreachable' });
    }
  }, []);

  useEffect(() => {
    mountedRef.current = true;
    void probe();
    return () => {
      mountedRef.current = false;
    };
  }, [probe]);

  const handleRecheck = useCallback(async () => {
    if (isRechecking) return;
    setIsRechecking(true);
    try {
      await probe();
    } finally {
      if (mountedRef.current) {
        setIsRechecking(false);
      }
    }
  }, [isRechecking, probe]);

  const ollamaConnected = setupState?.state === 'no_models_installed';
  const isWaitingForOllama = setupState?.state === 'ollama_unreachable';
  const isProbing = setupState === null;
  const stepOneActive = isWaitingForOllama;
  const stepOneDone = ollamaConnected;
  const stepTwoActive = ollamaConnected;

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
          overflow: 'hidden',
        }}
      >
        {/* Top edge highlight, identical to PermissionsStep / IntroStep. */}
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

        <h1
          style={{
            textAlign: 'center',
            fontSize: 22,
            fontWeight: 700,
            color: '#f0f0f2',
            letterSpacing: '-0.4px',
            lineHeight: 1.2,
            margin: '0 0 6px',
          }}
        >
          Set up your local AI
        </h1>
        <p
          style={{
            textAlign: 'center',
            fontSize: 13,
            color: 'rgba(255,255,255,0.55)',
            lineHeight: 1.55,
            margin: '0 auto 20px',
            maxWidth: 320,
          }}
        >
          {isProbing
            ? 'Checking your local Ollama setup…'
            : ollamaConnected
              ? 'Almost there. Pick a model so Thuki has something to think with. You can always switch to a different model later.'
              : 'Two quick things and you are in. Thuki runs Ollama locally so your conversations never leave this machine.'}
        </p>

        <div
          style={{
            display: isProbing ? 'none' : 'flex',
            flexDirection: 'column',
            gap: 10,
            marginBottom: 18,
          }}
        >
          <StepCard active={stepOneActive} done={stepOneDone}>
            <StepIcon
              variant={
                stepOneDone ? 'done' : stepOneActive ? 'active' : 'waiting'
              }
            >
              <ShieldCheckGlyph />
            </StepIcon>
            <StepText
              eyebrow={
                stepOneDone
                  ? 'STEP 1 · DONE'
                  : stepOneActive
                    ? 'STEP 1 · ACTION NEEDED'
                    : 'STEP 1'
              }
              eyebrowVariant={
                stepOneDone ? 'done' : stepOneActive ? 'active' : 'waiting'
              }
              title={
                stepOneDone ? 'Ollama is running' : 'Install & start Ollama'
              }
              titleMuted={!stepOneDone && !stepOneActive}
            />
            {stepOneDone ? <Badge color="green">Connected</Badge> : null}
          </StepCard>

          {stepOneActive ? (
            <ActionRow>
              <ActionCard
                title="Install Ollama"
                desc="brew install ollama"
                primary
                onClick={() => void copyToClipboard('brew install ollama')}
                buttonLabel="Copy"
                buttonGlyph={<CopyGlyph />}
              />
              <ActionCard
                title="Already installed?"
                desc="open -a Ollama"
                onClick={() => void copyToClipboard('open -a Ollama')}
                buttonLabel="Copy"
                buttonGlyph={<CopyGlyph />}
              />
            </ActionRow>
          ) : null}

          <StepCard active={stepTwoActive} done={false}>
            <StepIcon
              variant={
                stepTwoActive ? 'active' : stepOneDone ? 'active' : 'waiting'
              }
            >
              <CubeGlyph />
            </StepIcon>
            <StepText
              eyebrow={
                stepTwoActive ? 'STEP 2 · ACTION NEEDED' : 'STEP 2 · WAITING'
              }
              eyebrowVariant={stepTwoActive ? 'active' : 'waiting'}
              title="Pull a starter model"
              titleMuted={!stepTwoActive}
            />
          </StepCard>

          {stepTwoActive ? (
            <div
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: 8,
              }}
            >
              {RECOMMENDED_MODELS.map((m) => (
                <ModelCard
                  key={m.slug}
                  slug={m.slug}
                  description={m.description}
                  recommended={m.recommended === true}
                  onCopy={() => void copyToClipboard(buildPullCommand(m.slug))}
                />
              ))}
            </div>
          ) : null}
        </div>

        <button
          onClick={() => void handleRecheck()}
          aria-label="Re-check setup"
          disabled={isRechecking}
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
            cursor: isRechecking ? 'wait' : 'pointer',
            letterSpacing: '-0.1px',
            boxShadow: '0 4px 20px rgba(255,100,40,0.28)',
            textAlign: 'center',
            opacity: isRechecking ? 0.85 : 1,
          }}
        >
          {isRechecking ? 'Re-checking…' : 'Re-check setup'}
        </button>

        <p
          style={{
            textAlign: 'center',
            fontSize: 11,
            color: 'rgba(255,255,255,0.18)',
            marginTop: 14,
            lineHeight: 1.5,
          }}
        >
          Private by default · All inference runs on your machine
        </p>
      </motion.div>
    </div>
  );
}

// ─── Sub-components ──────────────────────────────────────────────────────────

type Variant = 'active' | 'done' | 'waiting';

interface StepIconProps {
  variant: Variant;
  children: React.ReactNode;
}

function StepIcon({ variant, children }: StepIconProps) {
  const palette: Record<
    Variant,
    { bg: string; border: string; color: string }
  > = {
    active: {
      bg: 'rgba(255,141,92,0.12)',
      border: 'rgba(255,141,92,0.25)',
      color: '#ff8d5c',
    },
    done: {
      bg: 'rgba(34,197,94,0.12)',
      border: 'rgba(34,197,94,0.2)',
      color: '#22c55e',
    },
    waiting: {
      bg: 'rgba(255,255,255,0.04)',
      border: 'rgba(255,255,255,0.08)',
      color: 'rgba(255,255,255,0.4)',
    },
  };
  const p = palette[variant];
  return (
    <div
      style={{
        width: 36,
        height: 36,
        borderRadius: 10,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        background: p.bg,
        border: `1px solid ${p.border}`,
        color: p.color,
      }}
    >
      {children}
    </div>
  );
}

interface StepTextProps {
  eyebrow: string;
  eyebrowVariant: Variant;
  title: string;
  titleMuted: boolean;
}

function StepText({
  eyebrow,
  eyebrowVariant,
  title,
  titleMuted,
}: StepTextProps) {
  const eyebrowColor: Record<Variant, string> = {
    active: 'rgba(255,141,92,0.8)',
    done: 'rgba(34,197,94,0.8)',
    waiting: 'rgba(255,255,255,0.4)',
  };
  return (
    <div style={{ flex: 1, minWidth: 0 }}>
      <div
        style={{
          fontSize: 10,
          fontWeight: 700,
          letterSpacing: 1.4,
          color: eyebrowColor[eyebrowVariant],
          margin: '0 0 2px',
        }}
      >
        {eyebrow}
      </div>
      <p
        style={{
          fontSize: 14,
          fontWeight: 600,
          color: titleMuted ? 'rgba(255,255,255,0.55)' : '#f0f0f2',
          margin: 0,
          letterSpacing: '-0.1px',
          lineHeight: 1.3,
        }}
      >
        {title}
      </p>
    </div>
  );
}

function ActionRow({ children }: { children: React.ReactNode }) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {children}
    </div>
  );
}

interface ActionCardProps {
  title: string;
  desc: string;
  primary?: boolean;
  onClick: () => void;
  buttonLabel: string;
  buttonGlyph: React.ReactNode;
}

function ActionCard({
  title,
  desc,
  primary,
  onClick,
  buttonLabel,
  buttonGlyph,
}: ActionCardProps) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 10,
        padding: '10px 12px',
        background: 'rgba(0,0,0,0.18)',
        border: '1px solid rgba(255,255,255,0.06)',
        borderRadius: 10,
      }}
    >
      <div style={{ minWidth: 0 }}>
        <p
          style={{
            fontSize: 12.5,
            fontWeight: 600,
            color: '#f0f0f2',
            margin: 0,
          }}
        >
          {title}
        </p>
        <p
          style={{
            fontSize: 11,
            color: 'rgba(255,255,255,0.45)',
            margin: '2px 0 0',
            fontFamily: '"SF Mono", Menlo, monospace',
          }}
        >
          {desc}
        </p>
      </div>
      <button
        onClick={onClick}
        aria-label={`${buttonLabel} ${title.toLowerCase()} command`}
        style={{
          flexShrink: 0,
          display: 'inline-flex',
          alignItems: 'center',
          gap: 5,
          padding: '5px 9px',
          borderRadius: 7,
          background: primary
            ? 'rgba(255,141,92,0.12)'
            : 'rgba(255,255,255,0.06)',
          border: `1px solid ${primary ? 'rgba(255,141,92,0.28)' : 'rgba(255,255,255,0.12)'}`,
          color: primary ? '#ff8d5c' : 'rgba(255,255,255,0.7)',
          fontSize: 10.5,
          fontWeight: 600,
          fontFamily: 'inherit',
          cursor: 'pointer',
        }}
      >
        {buttonGlyph}
        {buttonLabel}
      </button>
    </div>
  );
}

interface ModelCardProps {
  slug: string;
  description: string;
  recommended: boolean;
  onCopy: () => void;
}

function ModelCard({ slug, description, recommended, onCopy }: ModelCardProps) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 10,
        padding: '10px 12px',
        background: recommended
          ? 'radial-gradient(ellipse 100% 100% at 0% 0%, rgba(255,141,92,0.08) 0%, transparent 70%), rgba(0,0,0,0.18)'
          : 'rgba(0,0,0,0.18)',
        border: `1px solid ${recommended ? 'rgba(255,141,92,0.25)' : 'rgba(255,255,255,0.06)'}`,
        borderRadius: 10,
      }}
    >
      <div style={{ minWidth: 0 }}>
        {recommended ? (
          <div
            style={{
              fontSize: 8.5,
              fontWeight: 700,
              letterSpacing: 1.3,
              color: '#ff8d5c',
              margin: '0 0 3px',
            }}
          >
            RECOMMENDED
          </div>
        ) : null}
        <p
          style={{
            fontFamily: '"SF Mono", Menlo, monospace',
            fontSize: 13,
            fontWeight: 500,
            color: '#f0f0f2',
            margin: 0,
          }}
        >
          {slug}
        </p>
        <p
          style={{
            fontSize: 11,
            color: 'rgba(255,255,255,0.45)',
            margin: '2px 0 0',
          }}
        >
          {description}
        </p>
      </div>
      <button
        onClick={onCopy}
        aria-label={`Copy install command for ${slug}`}
        style={{
          flexShrink: 0,
          display: 'inline-flex',
          alignItems: 'center',
          gap: 5,
          padding: '5px 9px',
          borderRadius: 7,
          background: recommended
            ? 'rgba(255,141,92,0.12)'
            : 'rgba(255,255,255,0.06)',
          border: `1px solid ${recommended ? 'rgba(255,141,92,0.28)' : 'rgba(255,255,255,0.12)'}`,
          color: recommended ? '#ff8d5c' : 'rgba(255,255,255,0.7)',
          fontSize: 10.5,
          fontWeight: 600,
          fontFamily: 'inherit',
          cursor: 'pointer',
        }}
      >
        <CopyGlyph />
        Copy
      </button>
    </div>
  );
}

// ─── Glyphs ──────────────────────────────────────────────────────────────────

function ShieldCheckGlyph() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <path
        d="M9 1.5l6 3v4.5c0 3.5-2.5 6.5-6 7.5-3.5-1-6-4-6-7.5V4.5l6-3z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <path
        d="M6.5 9l2 2 3.5-4"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function CubeGlyph() {
  return (
    <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
      <path
        d="M9 1.5L2.5 5v8L9 16.5 15.5 13V5L9 1.5z"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path
        d="M2.5 5L9 8.5 15.5 5M9 16.5V8.5"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function CopyGlyph() {
  return (
    <svg width="11" height="11" viewBox="0 0 16 16" fill="none">
      <rect
        x="4.5"
        y="4.5"
        width="8"
        height="9"
        rx="1.5"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <path
        d="M3 11V3.5A1.5 1.5 0 0 1 4.5 2H10"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}
