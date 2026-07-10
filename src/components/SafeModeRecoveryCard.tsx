import { motion } from 'framer-motion';
import { useEffect, useRef } from 'react';
import thukiLogo from '../../src-tauri/icons/128x128.png';
import { useFitOnboardingWindow } from '../hooks/useFitOnboardingWindow';

/** Stable id for the headline, referenced by the dialog's `aria-labelledby`. */
const HEADING_ID = 'safe-mode-recovery-heading';

interface SafeModeRecoveryCardProps {
  /** Display name of the model that was loading when the previous launch never reached a healthy state. */
  modelName: string;
  /** Estimated resident footprint of that model, in GB, formatted to one decimal place. */
  sizeGb: string;
  /** Dismisses this screen and opens the model picker so the user can pick a different model. */
  onChooseDifferentModel: () => void;
  /** Dismisses this screen without switching models; the next chat message loads the model through the normal, ungated path. */
  onLoadAnyway: () => void;
}

/**
 * Recovery screen shown after the launch circuit breaker (issue #296) trips:
 * the previous launch(es) froze the machine while auto-loading a model and
 * never reached a responsive state, so this launch skipped that dangerous
 * auto-load and shows this instead. Model-led layout (no step checklist),
 * mirroring the visual shell of `PermissionsStep`/`IntroStep`.
 */
export function SafeModeRecoveryCard({
  modelName,
  sizeGb,
  onChooseDifferentModel,
  onLoadAnyway,
}: SafeModeRecoveryCardProps) {
  const cardRef = useRef<HTMLDivElement>(null);
  // Match the transparent window to the card, same as the onboarding steps
  // this screen mirrors; the card's content is fully known before it ever
  // mounts (the resolution effect in App.tsx only flips this screen on once
  // the model name and size are resolved), so there is no later reflow to
  // key a re-fit on.
  useFitOnboardingWindow(cardRef, null);

  // Thuki is summoned by a keyboard hotkey, so WebKit's :focus-visible
  // heuristic treats the interaction as keyboard-originated: whichever
  // element receives focus at mount paints a UA focus ring. Focusing the
  // dialog container (a tabindex="-1" programmatic target, not a tab stop)
  // instead of the primary button keeps that ring off the button while
  // still moving focus into the dialog for screen readers and keyboard
  // traversal.
  useEffect(() => {
    cardRef.current?.focus();
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
        role="dialog"
        aria-modal="true"
        aria-labelledby={HEADING_ID}
        tabIndex={-1}
        initial={{ opacity: 0, scale: 0.97, y: 8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ type: 'spring', stiffness: 300, damping: 28 }}
        style={{
          width: 420,
          // Flex child of a centering wrapper: never shrink to the window
          // width during a mid-resize measure (see the picker card in
          // ModelCheckStep).
          flexShrink: 0,
          background:
            'radial-gradient(ellipse 80% 55% at 50% 0%, rgba(255,141,92,0.14) 0%, rgba(28,24,20,0.97) 60%), rgba(28,24,20,0.97)',
          border: '1px solid rgba(255, 141, 92, 0.2)',
          borderRadius: 24,
          padding: '32px 26px 26px',
          boxShadow: '0 0 40px rgba(255,100,40,0.07)',
          position: 'relative',
          // This is a programmatic focus target (tabIndex={-1}), not a
          // control the user tabs to, so its own UA focus ring is noise.
          outline: 'none',
        }}
      >
        {/* Logo doubles as the window drag handle, matching the other
            onboarding-style screens. */}
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
              margin: '0 auto 16px',
              pointerEvents: 'none',
            }}
          />
        </div>

        {/* Headline + body */}
        <div style={{ textAlign: 'center', marginBottom: 22 }}>
          <h1
            id={HEADING_ID}
            style={{
              fontSize: 21,
              fontWeight: 700,
              color: '#f0f0f2',
              letterSpacing: '-0.5px',
              lineHeight: 1.25,
              margin: '0 0 10px',
            }}
          >
            Recovered in Safe Mode
          </h1>
          <p
            style={{
              fontSize: 13,
              color: 'rgba(255,255,255,0.5)',
              lineHeight: 1.6,
              margin: 0,
            }}
          >
            {`${modelName} (${sizeGb} GB) was loading when the last session ended unexpectedly, possibly because it needed more memory than was available.`}
          </p>
        </div>

        {/* Primary CTA */}
        <button
          onClick={onChooseDifferentModel}
          aria-label="Choose a different model"
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
          Choose a different model
        </button>

        {/* Secondary, quiet CTA - no fill, so it reads as the lower-emphasis choice. */}
        <button
          onClick={onLoadAnyway}
          aria-label="Load last model anyway"
          style={{
            display: 'block',
            width: '100%',
            padding: '11px',
            marginTop: 6,
            background: 'transparent',
            color: 'rgba(240,240,242,0.5)',
            fontSize: 13,
            fontWeight: 600,
            border: 'none',
            borderRadius: 12,
            cursor: 'pointer',
            letterSpacing: '-0.1px',
            textAlign: 'center',
          }}
        >
          Load last model anyway
        </button>
      </motion.div>
    </div>
  );
}
