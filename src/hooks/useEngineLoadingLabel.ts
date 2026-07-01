/**
 * Drives the cold-start loading label shown next to the chat's typing dots
 * while a local model provider (built-in engine or Ollama) is still
 * spinning up. A remote (`openai`-kind) provider has no local spin-up to
 * narrate, so it always renders `null`.
 *
 * Waits under `ENGINE_LOADING_THRESHOLD_MS` never show a label at all (a
 * warm/fast turn looks identical to today: bare dots) - matching the
 * standard "don't show a loading indicator for sub-second waits" guidance.
 * Past that threshold, phase 1's filler plays on a fixed schedule
 * (`ENGINE_PHASE1_PHRASES`). The built-in engine's real `warming` signal can
 * cut phase 1 short at any point and moves straight to phase 2
 * (`ENGINE_PHASE2_PHRASES`), which has its own independent schedule timed
 * from the moment warming started - not from when the turn began.
 *
 * Progress is monotonic within one active turn: once phase 2 has been
 * entered, the label never falls back to a phase-1 phrase, even if
 * `warming` later flips back to `false` while `active` is still `true`. In
 * practice the built-in engine's `warmup:builtin-warmed` event (its prime
 * finishing) can arrive before this specific request's first token does,
 * and without this guard the label would wrongly imply the model started
 * spinning up again.
 *
 * @param active Whether a turn is currently waiting on its first token from
 *   a local provider (i.e. the same condition that shows the typing dots).
 * @param providerKind The active provider's `kind` (`builtin` | `ollama` |
 *   `openai`).
 * @param warming Live `warmup:builtin-warming` state from
 *   {@link import('./useEngineWarmupStatus').useEngineWarmupStatus}. Only
 *   ever `true` for the built-in engine; Ollama never reaches phase 2.
 */

import { useEffect, useRef, useState } from 'react';
import {
  ENGINE_LOADING_THRESHOLD_MS,
  ENGINE_PHASE1_INTERVAL_MS,
  ENGINE_PHASE1_PHRASES,
  ENGINE_PHASE2_INTERVAL_MS,
  ENGINE_PHASE2_PHRASES,
} from '../config/engineLoadingLabels';

const LOCAL_PROVIDER_KINDS = new Set(['builtin', 'ollama']);

type Phase = 'idle' | 'phase1' | 'phase2';

export function useEngineLoadingLabel(
  active: boolean,
  providerKind: string,
  warming: boolean,
): string | null {
  const [label, setLabel] = useState<string | null>(null);
  const phaseRef = useRef<Phase>('idle');
  const timersRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  const clearTimers = () => {
    timersRef.current.forEach(clearTimeout);
    timersRef.current = [];
  };

  // Enters (or no-ops if already in) phase 2: cancels any pending phase-1
  // timers and schedules phase 2's own independent sequence, timed from
  // now rather than from when the turn began.
  const enterPhase2 = () => {
    if (phaseRef.current === 'phase2') return;
    phaseRef.current = 'phase2';
    clearTimers();
    // eslint-disable-next-line @eslint-react/set-state-in-effect -- intended: the real warming signal must override whatever phase-1 filler is showing the instant it fires
    setLabel(ENGINE_PHASE2_PHRASES[0]);
    timersRef.current.push(
      setTimeout(
        () => setLabel(ENGINE_PHASE2_PHRASES[1]),
        ENGINE_PHASE2_INTERVAL_MS,
      ),
    );
  };

  // Turn lifecycle: (re)starts phase 1 when the turn becomes active for a
  // local provider, tears everything down when it ends. Deliberately does
  // NOT depend on `warming` - a warming flip must never restart this effect,
  // or an in-progress phase 2 would be clobbered back to phase 1's schedule.
  useEffect(() => {
    if (!active || !LOCAL_PROVIDER_KINDS.has(providerKind)) {
      phaseRef.current = 'idle';
      clearTimers();
      // eslint-disable-next-line @eslint-react/set-state-in-effect -- intended: an inactive/remote turn always renders dots-only
      setLabel(null);
      return;
    }

    phaseRef.current = 'phase1';
    // eslint-disable-next-line @eslint-react/set-state-in-effect -- intended: a fresh turn always starts dots-only until the threshold elapses
    setLabel(null);
    timersRef.current = ENGINE_PHASE1_PHRASES.map((phrase, i) =>
      // eslint-disable-next-line @eslint-react/web-api-no-leaked-timeout -- cleared via clearTimers() on the next transition
      setTimeout(
        () => setLabel(phrase),
        ENGINE_LOADING_THRESHOLD_MS + i * ENGINE_PHASE1_INTERVAL_MS,
      ),
    );

    return clearTimers;
  }, [active, providerKind]);

  // Reacts to the real warming signal the instant it fires, independent of
  // how far phase 1's timer has progressed.
  useEffect(() => {
    if (!active || !LOCAL_PROVIDER_KINDS.has(providerKind)) return;
    if (warming) enterPhase2();
    // eslint-disable-next-line @eslint-react/exhaustive-deps -- intended: enterPhase2 reads refs/setLabel only, it does not need to be a dep
  }, [warming, active, providerKind]);

  return label;
}
