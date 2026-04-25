/**
 * Onboarding step that gates the chat overlay on a working local Ollama
 * setup with at least one installed model.
 *
 * Layout:
 *   - Vertical timeline rail with numbered nodes connected by a thin line.
 *   - Step 1 active shows a single title row, then a two-tab install hero
 *     (Install Ollama / Already Installed?) above a single code box that
 *     swaps its command per tab. A short sub-line below the box invites
 *     the user to paste the command or visit the Ollama docs.
 *   - Step 2 active hosts a compact list of starter models, all rendered
 *     equal — no badge, no hierarchy. The user picks whichever fits.
 *
 * Probes Ollama via the `check_model_setup` Tauri command on mount and on
 * every Re-check click. Background polling is intentionally absent so
 * idle CPU and IPC stay at zero between explicit user actions.
 */

import { AnimatePresence, motion } from 'framer-motion';
import type React from 'react';
import { useState, useEffect, useRef, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import thukiLogo from '../../../src-tauri/icons/128x128.png';
import { useConfig } from '../../contexts/ConfigContext';
import { Badge } from './_shared';

const OLLAMA_DOCS_URL = 'https://ollama.com/download';
const OLLAMA_SEARCH_URL = 'https://ollama.com/search';

/**
 * Extracts the `host:port` segment from an Ollama daemon URL for display.
 * Falls back to the raw input when the URL cannot be parsed (e.g. user
 * config holds a non-URL string), so the UI never shows a confusing
 * empty value.
 */
function formatListenAddr(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

type ModelSetupState =
  | { state: 'ollama_unreachable' }
  | { state: 'no_models_installed' }
  | { state: 'ready'; active_slug: string; installed: string[] };

interface InstallTab {
  id: string;
  label: string;
  command: string;
}

/**
 * Install routes shown above the Step 1 code box. The first entry is the
 * default selection. `command` is the exact string copied to the
 * clipboard when the copy pill is clicked.
 */
const INSTALL_TABS: InstallTab[] = [
  {
    id: 'install',
    label: 'Install Ollama',
    command: 'curl -fsSL https://ollama.com/install.sh | sh',
  },
  {
    id: 'already-installed',
    label: 'Already Installed?',
    command: 'open -a Ollama',
  },
];

/**
 * Starter models offered in Step 2. All entries support text and image
 * input (vision / multimodal). Sizes are pulled from the official Ollama
 * library (ollama.com/library) and reflect the default tag at time of
 * authoring. All entries are intentionally peers — no recommended
 * badge — so the user picks whichever fits their hardware.
 */
const STARTER_MODELS: Array<{
  slug: string;
  description: string;
  size: string;
}> = [
  { slug: 'gemma4:e4b', description: 'Google · vision', size: '9.6 GB' },
  {
    slug: 'llama3.2-vision:11b',
    description: 'Meta · vision',
    size: '7.8 GB',
  },
  { slug: 'phi4:14b', description: 'Microsoft · text', size: '9.1 GB' },
];

/**
 * Builds the public Ollama library URL for a model slug. Drops the `:tag`
 * suffix so the destination shows every available variant rather than
 * pinning the user to one quantisation. Both `gemma4` and `gemma4:e4b`
 * resolve, but the bare-name URL is the more useful landing.
 */
function buildOllamaLibraryUrl(slug: string): string {
  const base = slug.split(':')[0];
  return `https://ollama.com/library/${base}`;
}

function buildPullCommand(slug: string): string {
  return `ollama pull ${slug}`;
}

async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export function ModelCheckStep() {
  const [setupState, setSetupState] = useState<ModelSetupState | null>(null);
  const [isRechecking, setIsRechecking] = useState(false);
  const mountedRef = useRef(true);

  const probe = useCallback(async () => {
    try {
      const next = await invoke<ModelSetupState>('check_model_setup');
      if (!mountedRef.current) return;
      if (next.state === 'ready') {
        await invoke('advance_past_model_check');
        return;
      }
      setSetupState(next);
    } catch {
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
    setIsRechecking(true);
    try {
      await probe();
    } finally {
      if (mountedRef.current) {
        setIsRechecking(false);
      }
    }
  }, [probe]);

  const ollamaConnected = setupState?.state === 'no_models_installed';
  const isWaitingForOllama = setupState?.state === 'ollama_unreachable';
  const isProbing = setupState === null;

  const titleSub = isProbing
    ? 'Checking your local Ollama setup…'
    : ollamaConnected
      ? "Almost there. Let's pick a brain for Thuki."
      : 'Runs Ollama locally. Your chats stay on this machine.';

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
          padding: '26px 22px 22px',
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
          style={{ textAlign: 'center', marginBottom: 12, cursor: 'grab' }}
        >
          <img
            src={thukiLogo}
            width={40}
            height={40}
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
            fontSize: 18,
            fontWeight: 700,
            color: '#f0f0f2',
            letterSpacing: '-0.3px',
            lineHeight: 1.25,
            margin: '0 0 4px',
          }}
        >
          Set up your local AI
        </h1>
        <p
          style={{
            textAlign: 'center',
            fontSize: 12.5,
            color: 'rgba(255,255,255,0.55)',
            lineHeight: 1.5,
            margin: '0 auto 18px',
            maxWidth: 320,
          }}
        >
          {titleSub}
        </p>

        {!isProbing ? (
          <Rail
            stepOneActive={isWaitingForOllama}
            stepOneDone={ollamaConnected}
            stepTwoActive={ollamaConnected}
          />
        ) : null}

        <button
          onClick={() => void handleRecheck()}
          aria-label="Verify setup"
          disabled={isRechecking}
          style={{
            display: 'block',
            width: '100%',
            padding: '11px',
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
            marginTop: 4,
          }}
        >
          {isRechecking ? 'Verifying…' : 'Verify setup'}
        </button>

        <p
          style={{
            textAlign: 'center',
            fontSize: 11,
            color: 'rgba(255,255,255,0.18)',
            marginTop: 12,
            lineHeight: 1.5,
          }}
        >
          Private by default · All inference runs on your machine
        </p>
      </motion.div>
    </div>
  );
}

// ─── Rail ────────────────────────────────────────────────────────────────────

interface RailProps {
  stepOneActive: boolean;
  stepOneDone: boolean;
  stepTwoActive: boolean;
}

/**
 * Two-step vertical timeline. The connecting line is rendered once as an
 * absolute element behind the node column so it spans the full rail
 * regardless of how tall each row's content grows.
 */
function Rail({ stepOneActive, stepOneDone, stepTwoActive }: RailProps) {
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '24px 1fr',
        columnGap: 12,
        position: 'relative',
        marginBottom: 16,
      }}
    >
      <div
        aria-hidden="true"
        style={{
          position: 'absolute',
          top: 14,
          bottom: 14,
          left: 11,
          width: 1,
          background:
            'linear-gradient(180deg, rgba(255,141,92,0.25), rgba(255,255,255,0.04))',
        }}
      />

      <RailNode number={1} variant={stepOneDone ? 'done' : 'active'} />
      <RowOne active={stepOneActive} done={stepOneDone} />

      <RailNode
        number={2}
        variant={stepTwoActive ? 'active' : 'wait'}
        topGap={20}
      />
      <RowTwo active={stepTwoActive} />
    </div>
  );
}

type NodeVariant = 'active' | 'done' | 'wait';

interface RailNodeProps {
  number: number;
  variant: NodeVariant;
  topGap?: number;
}

function RailNode({ number, variant, topGap = 0 }: RailNodeProps) {
  const palette: Record<
    NodeVariant,
    { bg: string; border: string; color: string }
  > = {
    active: {
      bg: 'rgba(255,141,92,0.1)',
      border: 'rgba(255,141,92,0.4)',
      color: '#ff8d5c',
    },
    done: {
      bg: 'rgba(34,197,94,0.12)',
      border: 'rgba(34,197,94,0.4)',
      color: '#22c55e',
    },
    wait: {
      bg: 'rgba(255,255,255,0.03)',
      border: 'rgba(255,255,255,0.1)',
      color: 'rgba(255,255,255,0.4)',
    },
  };
  const p = palette[variant];
  return (
    <div
      style={{
        gridColumn: 1,
        display: 'flex',
        justifyContent: 'center',
        alignItems: 'flex-start',
        marginTop: topGap,
        zIndex: 1,
      }}
    >
      <div
        style={{
          width: 24,
          height: 24,
          borderRadius: 999,
          background: p.bg,
          border: `1px solid ${p.border}`,
          color: p.color,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          fontSize: 10.5,
          fontWeight: 700,
          letterSpacing: '-0.2px',
        }}
      >
        {variant === 'done' ? '✓' : number}
      </div>
    </div>
  );
}

// ─── Row 1: install Ollama ───────────────────────────────────────────────────

interface RowOneProps {
  active: boolean;
  done: boolean;
}

function RowOne({ active, done }: RowOneProps) {
  const config = useConfig();
  const [selectedTabIdx, setSelectedTabIdx] = useState(0);
  const tab = INSTALL_TABS[selectedTabIdx];

  return (
    <div style={{ gridColumn: 2, marginBottom: 4 }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 10,
        }}
      >
        <div style={{ minWidth: 0 }}>
          <p
            style={{
              fontSize: 14,
              fontWeight: 600,
              color: '#f0f0f2',
              margin: 0,
              letterSpacing: '-0.1px',
              lineHeight: 1.3,
            }}
          >
            {done ? 'Ollama is running' : 'Install & start Ollama'}
          </p>
          {done ? (
            <p
              style={{
                fontFamily: '"SF Mono", Menlo, monospace',
                fontSize: 10.5,
                color: 'rgba(255,255,255,0.4)',
                margin: '3px 0 0',
                letterSpacing: '-0.1px',
              }}
            >
              Listening on {formatListenAddr(config.model.ollamaUrl)}
            </p>
          ) : null}
        </div>
        {done ? <Badge color="green">live</Badge> : null}
      </div>

      {active ? (
        <>
          <div
            style={{
              marginTop: 10,
              padding: 10,
              background: 'rgba(0, 0, 0, 0.3)',
              border: '1px solid rgba(255, 255, 255, 0.05)',
              borderRadius: 12,
            }}
          >
            <div
              style={{
                display: 'flex',
                gap: 4,
                marginBottom: 8,
              }}
            >
              {INSTALL_TABS.map((t, i) => (
                <TabButton
                  key={t.id}
                  label={t.label}
                  selected={i === selectedTabIdx}
                  onClick={() => setSelectedTabIdx(i)}
                />
              ))}
            </div>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 8,
                padding: '8px 10px',
                background: 'rgba(0, 0, 0, 0.32)',
                border: '1px solid rgba(255, 255, 255, 0.05)',
                borderRadius: 10,
                height: 52,
                boxSizing: 'border-box',
                overflow: 'hidden',
              }}
            >
              <span
                style={{
                  fontFamily: '"SF Mono", Menlo, monospace',
                  fontSize: 11.5,
                  color: '#f0f0f2',
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-all',
                  flex: 1,
                  minWidth: 0,
                  lineHeight: 1.4,
                }}
              >
                <span
                  style={{
                    color: 'rgba(255,141,92,0.75)',
                    marginRight: 6,
                  }}
                >
                  $
                </span>
                {tab.command}
              </span>
              <CopyButton
                command={tab.command}
                ariaLabel={`Copy ${tab.label.toLowerCase()} command`}
                iconOnly
              />
            </div>
          </div>
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              flexWrap: 'wrap',
              gap: 5,
              marginTop: 8,
              fontSize: 11,
              lineHeight: 1.5,
            }}
          >
            <span style={{ color: 'rgba(255,255,255,0.42)' }}>
              Paste this in Terminal or visit
            </span>
            <DocsLink
              ariaLabel="Open Ollama documentation"
              url={OLLAMA_DOCS_URL}
            >
              Ollama docs ↗
            </DocsLink>
          </div>
        </>
      ) : null}
    </div>
  );
}

// ─── Row 2: pull a starter model ─────────────────────────────────────────────

function RowTwo({ active }: { active: boolean }) {
  return (
    <div style={{ gridColumn: 2, marginTop: 20 }}>
      <p
        style={{
          fontSize: 14,
          fontWeight: 600,
          color: active ? '#f0f0f2' : 'rgba(255,255,255,0.55)',
          margin: 0,
          letterSpacing: '-0.1px',
          lineHeight: 1.3,
        }}
      >
        Pull a starter model
      </p>

      {active ? (
        <>
          <p
            style={{
              fontSize: 11.5,
              color: 'rgba(255,255,255,0.45)',
              margin: '3px 0 0',
              lineHeight: 1.5,
            }}
          >
            You can swap or add more later.
          </p>
          <div
            style={{
              marginTop: 10,
              border: '1px solid rgba(255, 255, 255, 0.05)',
              borderRadius: 12,
              overflow: 'hidden',
              background: 'rgba(0, 0, 0, 0.18)',
            }}
          >
            {STARTER_MODELS.map((m, i) => (
              <ModelRow
                key={m.slug}
                slug={m.slug}
                description={m.description}
                size={m.size}
                isLast={i === STARTER_MODELS.length - 1}
              />
            ))}
          </div>

          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              gap: 2,
              marginTop: 10,
              fontSize: 11,
              lineHeight: 1.5,
            }}
          >
            <span style={{ color: 'rgba(255,255,255,0.42)' }}>
              Paste the command in Terminal
            </span>
            <span style={{ color: 'rgba(255,255,255,0.28)' }}>or</span>
            <DocsLink
              ariaLabel="Browse all models on Ollama"
              url={OLLAMA_SEARCH_URL}
            >
              Browse all models on ollama.com ↗
            </DocsLink>
          </div>
        </>
      ) : null}
    </div>
  );
}

interface ModelRowProps {
  slug: string;
  description: string;
  size: string;
  isLast: boolean;
}

function ModelRow({ slug, description, size, isLast }: ModelRowProps) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 10,
        padding: '9px 12px',
        borderBottom: isLast ? 'none' : '1px solid rgba(255, 255, 255, 0.04)',
      }}
    >
      <div style={{ minWidth: 0 }}>
        <SlugLink slug={slug} />
        <p
          style={{
            fontSize: 10.5,
            color: 'rgba(255,255,255,0.45)',
            margin: '2px 0 0',
          }}
        >
          {description} · {size}
        </p>
      </div>
      <CopyButton
        command={buildPullCommand(slug)}
        ariaLabel={`Copy install command for ${slug}`}
      />
    </div>
  );
}

/**
 * Renders the model slug as an inline button styled like text. Click
 * opens the model's Ollama library page in the user's default browser
 * via the `open_url` Tauri command. Hover lifts the slug to brand
 * orange with a subtle underline so it reads as discoverable without
 * shouting.
 */
function SlugLink({ slug }: { slug: string }) {
  const [hover, setHover] = useState(false);
  return (
    <button
      onClick={() =>
        void invoke('open_url', { url: buildOllamaLibraryUrl(slug) })
      }
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-label={`Open ${slug} on Ollama`}
      style={{
        display: 'block',
        background: 'transparent',
        border: 'none',
        padding: 0,
        margin: 0,
        fontFamily: '"SF Mono", Menlo, monospace',
        fontSize: 12.5,
        fontWeight: 500,
        color: hover ? '#ff8d5c' : '#f0f0f2',
        textDecoration: hover ? 'underline' : 'none',
        textDecorationColor: 'rgba(255,141,92,0.5)',
        textUnderlineOffset: 3,
        cursor: 'pointer',
        userSelect: 'text',
        textAlign: 'left',
        transition: 'color 160ms ease',
      }}
    >
      {slug}
    </button>
  );
}

// ─── Tab + copy + docs link ──────────────────────────────────────────────────

interface DocsLinkProps {
  ariaLabel: string;
  url: string;
  children: React.ReactNode;
}

function DocsLink({ ariaLabel, url, children }: DocsLinkProps) {
  const [hover, setHover] = useState(false);
  return (
    <button
      onClick={() => void invoke('open_url', { url })}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-label={ariaLabel}
      style={{
        background: 'transparent',
        border: 'none',
        padding: 0,
        fontFamily: 'inherit',
        fontSize: 11,
        fontWeight: 500,
        color: hover ? '#ff8d5c' : 'rgba(255,141,92,0.7)',
        cursor: 'pointer',
        transition: 'color 160ms ease',
      }}
    >
      {children}
    </button>
  );
}

interface TabButtonProps {
  label: string;
  selected: boolean;
  onClick: () => void;
}

function TabButton({ label, selected, onClick }: TabButtonProps) {
  const [hover, setHover] = useState(false);
  const borderColor = selected
    ? 'rgba(255, 141, 92, 0.28)'
    : hover
      ? 'rgba(255, 255, 255, 0.1)'
      : 'transparent';
  const bg = selected
    ? 'rgba(255, 141, 92, 0.1)'
    : hover
      ? 'rgba(255, 255, 255, 0.04)'
      : 'rgba(255, 255, 255, 0.025)';
  const color = selected
    ? '#ff8d5c'
    : hover
      ? 'rgba(255,255,255,0.85)'
      : 'rgba(255,255,255,0.55)';

  return (
    <button
      onClick={onClick}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-pressed={selected}
      style={{
        flex: 1,
        padding: '6px 8px',
        borderRadius: 8,
        fontSize: 11.5,
        fontWeight: 600,
        color,
        background: bg,
        border: `1px solid ${borderColor}`,
        cursor: 'pointer',
        fontFamily: 'inherit',
        transition: 'all 160ms ease',
      }}
    >
      {label}
    </button>
  );
}

const COPIED_RESET_MS = 1500;

interface CopyButtonProps {
  command: string;
  ariaLabel: string;
  label?: string;
  iconOnly?: boolean;
}

function CopyButton({
  command,
  ariaLabel,
  label = 'Copy',
  iconOnly = false,
}: CopyButtonProps) {
  const [hover, setHover] = useState(false);
  const [copied, setCopied] = useState(false);
  const timeoutRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  const handleClick = useCallback(async () => {
    const ok = await copyToClipboard(command);
    if (!ok) return;
    setCopied(true);
    if (timeoutRef.current !== null) {
      window.clearTimeout(timeoutRef.current);
    }
    timeoutRef.current = window.setTimeout(() => {
      setCopied(false);
      timeoutRef.current = null;
    }, COPIED_RESET_MS);
  }, [command]);

  const borderColor = copied
    ? 'rgba(34,197,94,0.55)'
    : hover
      ? 'rgba(255,141,92,0.55)'
      : 'rgba(255,255,255,0.12)';
  const labelColor =
    hover || copied ? 'rgba(255,255,255,0.95)' : 'rgba(255,255,255,0.7)';
  const glyphColor = copied
    ? '#22c55e'
    : hover
      ? '#ff8d5c'
      : 'rgba(255,255,255,0.7)';

  return (
    <button
      onClick={() => void handleClick()}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      aria-label={ariaLabel}
      style={{
        flexShrink: 0,
        display: 'inline-flex',
        alignItems: 'center',
        gap: 5,
        padding: iconOnly ? '5px 6px' : '5px 9px',
        borderRadius: 7,
        background: 'rgba(255,255,255,0.06)',
        border: `1px solid ${borderColor}`,
        color: labelColor,
        fontSize: 10.5,
        fontWeight: 600,
        fontFamily: 'inherit',
        cursor: 'pointer',
        transition:
          'border-color 160ms ease, color 160ms ease, background-color 160ms ease',
      }}
    >
      <AnimatePresence mode="wait" initial={false}>
        {copied ? (
          <motion.span
            key="copied"
            initial={{ opacity: 0, y: 2 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -2 }}
            transition={{ duration: 0.14 }}
            style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}
          >
            <span style={{ display: 'inline-flex', color: glyphColor }}>
              <CheckGlyph />
            </span>
            {iconOnly ? null : 'Copied'}
          </motion.span>
        ) : (
          <motion.span
            key="copy"
            initial={{ opacity: 0, y: 2 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -2 }}
            transition={{ duration: 0.14 }}
            style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}
          >
            <span style={{ display: 'inline-flex', color: glyphColor }}>
              <CopyGlyph />
            </span>
            {iconOnly ? null : label}
          </motion.span>
        )}
      </AnimatePresence>
    </button>
  );
}

// ─── Glyphs ──────────────────────────────────────────────────────────────────

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

function CheckGlyph() {
  return (
    <svg width="11" height="11" viewBox="0 0 16 16" fill="none">
      <path
        d="M3 8.5l3.2 3.2L13 5"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
