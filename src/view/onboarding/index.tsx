import { BuiltinAnnouncementStep } from './BuiltinAnnouncementStep';
import { IntroStep } from './IntroStep';
import { ModelCheckStep } from './ModelCheckStep';
import { PermissionsStep } from './PermissionsStep';
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
 */
export function OnboardingView({ stage, onComplete, downloadStatus }: Props) {
  if (stage === 'intro') {
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
