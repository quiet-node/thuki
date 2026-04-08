import { IntroStep } from './IntroStep';
import { PermissionsStep } from './PermissionsStep';

export type OnboardingStage = 'permissions' | 'intro';

interface Props {
  stage: OnboardingStage;
  onComplete: () => void;
}

/**
 * Onboarding module orchestrator.
 *
 * Renders the correct step based on the persisted onboarding stage emitted
 * by the backend at startup. The stage advances on the backend:
 *
 *   permissions -> (quit+reopen) -> intro -> complete (normal app)
 *
 * When stage is "complete" the backend never emits the onboarding event,
 * so this component is never rendered.
 */
export function OnboardingView({ stage, onComplete }: Props) {
  if (stage === 'intro') {
    return <IntroStep onComplete={onComplete} />;
  }
  return <PermissionsStep />;
}
