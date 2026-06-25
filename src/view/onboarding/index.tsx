import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

import { BuiltinAnnouncementStep } from './BuiltinAnnouncementStep';
import { IntroStep } from './IntroStep';
import { ModelCheckStep } from './ModelCheckStep';
import { PermissionsStep } from './PermissionsStep';
import { SubscribeStep } from './SubscribeStep';
import type { DownloadStripStatus } from '../../components/DownloadStatusStrip';

/**
 * Stage values mirror the Rust `OnboardingStage` enum exactly. The
 * backend emits these strings as the `stage` field on the
 * `thuki://onboarding` event; any drift here breaks the dispatch.
 */
export type OnboardingStage =
  | 'permissions'
  | 'builtin_announcement'
  | 'model_check'
  | 'intro';

interface Props {
  stage: OnboardingStage;
  onComplete: () => void;
  /** Ambient download status shown inside the intro card (intro stage only). */
  downloadStatus?: DownloadStripStatus | null;
}

/**
 * Onboarding module orchestrator.
 *
 * Renders the correct step based on the persisted onboarding stage emitted
 * by the backend at startup. The stage advances on the backend:
 *
 *   permissions -> (quit+reopen) -> model_check -> (advance) -> intro -> complete
 *
 * Upgraders take one extra step after permissions:
 *   permissions -> builtin_announcement -> model_check -> intro -> complete
 *
 * When stage is "complete" the backend never emits the onboarding event,
 * so this component is never rendered.
 *
 * The `intro` stage carries a frontend-only sub-step: the optional
 * roadmap/email screen (`SubscribeStep`) is shown once before the final tips
 * card. It is tracked in local state rather than as a backend stage because
 * skipping and subscribing both lead straight to the tips card, which owns
 * onboarding completion, so it needs no persistence.
 */
export function OnboardingView({ stage, onComplete, downloadStatus }: Props) {
  const [roadmapSeen, setRoadmapSeen] = useState(false);

  // Advance from the optional roadmap/email screen to the tips card. This is a
  // frontend-only swap, not a backend stage change, so nothing covers the
  // panel while the window resizes between the two different-height cards. Hide
  // the panel first so the resize happens off-screen; IntroStep's fit hook
  // fades it back in once the tips card has settled, matching the cross-fade of
  // every backend-driven onboarding transition.
  const advanceToTips = async () => {
    await invoke('set_overlay_alpha', { alpha: 0, durationMs: 0 });
    setRoadmapSeen(true);
  };

  if (stage === 'intro') {
    if (!roadmapSeen) {
      return (
        <SubscribeStep
          onContinue={() => void advanceToTips()}
          downloadStatus={downloadStatus}
        />
      );
    }
    return (
      <IntroStep onComplete={onComplete} downloadStatus={downloadStatus} />
    );
  }
  if (stage === 'model_check') {
    // ModelCheckStep advances to `intro` via the backend
    // `advance_past_model_check` command, which re-emits the onboarding
    // event. No callback wiring needed here.
    void onComplete; // referenced for parity; unused by ModelCheckStep
    return <ModelCheckStep />;
  }
  if (stage === 'builtin_announcement') {
    // Both branches advance to `model_check` via the backend
    // `advance_past_builtin_announcement` command, which re-emits the
    // onboarding event. No callback wiring needed here.
    return <BuiltinAnnouncementStep />;
  }
  return <PermissionsStep />;
}
