/**
 * Copy and timing for the cold-start loading label shown next to the 9-dot
 * typing indicator while a local model provider (built-in engine or Ollama)
 * is still spinning up. A warm/fast turn never reaches these: the label only
 * appears once a wait has run past `ENGINE_LOADING_THRESHOLD_MS`.
 *
 * Two real backend phases exist, and the copy is split to match:
 * - **Phase 1** (`engine:status = starting`): process spawn + reading the
 *   GGUF weights into RAM + health check. Usually brief.
 * - **Phase 2** (`warmup:builtin-warming` -> `-warmed`, built-in only): the
 *   model is resident and the system-prompt prefill is running. This is
 *   where real waits tend to land, so its filler describes generic "still
 *   going" rather than a specific mechanism (weights are already loaded by
 *   this point, so re-using "Reading model weights..." here would be false).
 *
 * Within each phase, the second phrase is elapsed-time filler, not real
 * progress - neither phase exposes a finer sub-signal, so elapsed time is
 * the only honest thing to communicate ("still going, and it's been a
 * while"). Ollama has no phase-2 signal at all, so it only ever shows phase
 * 1's phrases, holding on the last one.
 */

/** Minimum wait before any label appears; sub-threshold waits stay dots-only. */
export const ENGINE_LOADING_THRESHOLD_MS = 900;

/** Phase 1 filler: process spawning / weights loading / health-checking. */
export const ENGINE_PHASE1_PHRASES: readonly string[] = [
  'Starting up the model…',
  'Reading model weights…',
];

/** Spacing between phase 1's two phrases. */
export const ENGINE_PHASE1_INTERVAL_MS = 1500;

/** Phase 2 filler: the built-in engine's real prefill-priming signal. */
export const ENGINE_PHASE2_PHRASES: readonly string[] = [
  'Warming up…',
  'Bigger models take a little longer…',
];

/** Spacing between phase 2's two phrases. */
export const ENGINE_PHASE2_INTERVAL_MS = 3000;

/**
 * Shown when the engine was already `loaded` (not cold, not mid-prime) at
 * the moment the turn began, yet the wait still crosses the threshold - the
 * per-request prefill cost scales with how much conversation history there
 * is, so a warm engine can still take real seconds on a long conversation.
 * Neither phase-1 ("starting up") nor phase-2 ("warming up") language is
 * true here: the engine never left `loaded`, so this gets its own single
 * held phrase instead of borrowing either sequence's copy.
 */
export const ENGINE_SLOW_WARM_LABEL = 'Processing your message…';
