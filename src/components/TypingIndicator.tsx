import { ThreeDotMotion } from './ThreeDotMotion';

/**
 * Post-submit working indicator (unified with search/think/engine load).
 *
 * Historically a 9-dot spiral; now the locked Y1 three-dot motion used by
 * {@link RequestStatusStrip}. Kept as a named export so older imports and
 * mental models still resolve, but all new code should prefer RequestStatusStrip.
 */
export function TypingIndicator() {
  return (
    <div className="flex w-full justify-start py-1">
      <ThreeDotMotion />
    </div>
  );
}
