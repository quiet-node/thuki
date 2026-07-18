import type { EngineErrorKind } from '../hooks/useModel';

interface ErrorCardProps {
  kind: EngineErrorKind;
  message: string;
  /**
   * Opens the overlay model picker so an `EngineStartFailed` is never a dead
   * end. Wired only for that kind: it renders the "Switch model" recovery
   * button. Absent for every other kind (and in tests that do not exercise
   * recovery), where no button renders.
   */
  onSwitchModel?: () => void;
  /**
   * Replays the turn with the pre-load memory gate bypassed (issue #296).
   * Wired only for `InsufficientMemory`. `remember` carries the user's choice
   * between the split "Load once" (`false`) and "Always allow this model"
   * (`true`) actions, so the host can persist the per-model override on the
   * remember variant; in the freeze band only the single "Load anyway"
   * (`false`) force button is offered.
   */
  onLoadAnyway?: (remember: boolean) => void;
  /**
   * Machine-readable figures backing the `InsufficientMemory` card copy,
   * sourced from the `estimate_model_fit` command. When absent (the fetch
   * has not resolved yet, or failed), the card falls back to the generic
   * message-based render so nothing crashes.
   */
  insufficientMemoryInfo?: {
    modelName: string;
    requiredBytes: number;
    availableBytes: number;
    /**
     * Whether a per-model "remember" could suppress this warning (backend
     * `!is_freeze_band`). True in the mild band, where the card offers the
     * "Always allow this model" action; false in the freeze band, where only
     * the single "Load anyway" force is offered because the backend never
     * honors a remember for such a load. The single source of truth for the
     * freeze-floor fraction; the frontend keeps no magic number of its own.
     */
    canRemember: boolean;
  };
}

const barColors: Record<EngineErrorKind, string> = {
  EngineUnreachable: '#ef4444',
  // Same red as EngineUnreachable: a sidecar crash is equally severe.
  EngineStartFailed: '#ef4444',
  // Amber, not red: an unsupported model architecture is a "pick another
  // model" nudge, not an engine crash, so it shares the warning hue.
  ModelUnsupported: '#f59e0b',
  ModelNotFound: '#f59e0b',
  // Same accent as ModelNotFound: this is a configuration/setup nudge,
  // not a daemon failure, so the warning hue (amber) is the right read.
  NoModelSelected: '#f59e0b',
  // Same warning hue: a soft, force-overridable refusal, not a crash
  // (issue #296).
  InsufficientMemory: '#f59e0b',
  Other: 'rgba(255,255,255,0.2)',
};

/**
 * The consequence copy shown when a model may not fit in available memory
 * (issue #296): loading it anyway risks memory pressure severe enough to freeze
 * the machine. Exported so the ambient `AutoPrimeSkippedStrip` confirm step and
 * this in-chat `InsufficientMemory` card render byte-identical wording from one
 * source, rather than drifting two copies of a safety warning.
 */
export const INSUFFICIENT_MEMORY_CONSEQUENCE =
  'To fit this model, your Mac may compress memory, which can slow things down or, in extreme cases, freeze the entire machine and require a reboot.';

/**
 * The single freeze-band explainer, shown to everyone in that band. It speaks
 * only to severity: naming a saved setting explains nothing to a user who does
 * not recall choosing it months earlier, so this deliberately makes no mention
 * of any remembered override. Exported so the ambient `AutoPrimeSkippedStrip`
 * and this in-chat card render byte-identical wording from one source.
 */
export const MEMORY_FREEZE_NOTE =
  'That is far too tight to load on its own. Thuki always asks at this level, because loading can slow your Mac badly or freeze it.';

/**
 * Red "Memory critically low" severity tag that heads the freeze-band warning.
 * Exported as a component so both memory surfaces render identical severity
 * chrome rather than duplicating the token values.
 *
 * Text only: it carries no dot of its own, because each surface already shows
 * exactly one indicator in this band (the strip drops its status dot here, and
 * the card tints its accent bar red to match). Squared corners at 5px, just
 * under the card's own 6px radius, so it reads as a tag rather than a pill.
 */
export const MEMORY_CRITICAL_RED = '#f87171';

export function MemoryCriticalChip() {
  return (
    <span
      data-testid="memory-critical-chip"
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        fontSize: '10px',
        fontWeight: 700,
        letterSpacing: '0.08em',
        textTransform: 'uppercase',
        padding: '3px 8px',
        borderRadius: '5px',
        color: MEMORY_CRITICAL_RED,
        background: 'rgba(248, 113, 113, 0.12)',
        border: '1px solid rgba(248, 113, 113, 0.35)',
      }}
    >
      Memory critically low
    </span>
  );
}

/** Bytes per gigabyte, matching the Rust gate's `1u64 << 30` GiB divisor. */
const BYTES_PER_GB = 1024 ** 3;

/** Formats a byte count as a one-decimal GB string, matching Rust's `{:.1}`. */
function formatGb(bytes: number): string {
  return (bytes / BYTES_PER_GB).toFixed(1);
}

/** Fixed title for an engine start failure; the backend detail renders below
 *  it verbatim, so the title stays a stable, human heading regardless of the
 *  raw llama-server output. */
const ENGINE_START_FAILED_TITLE = "Thuki's engine couldn't start this model";

/** Outlined primary action button (transparent fill). Used by "Switch model"
 *  on other cards, and by "Load once" / "Load anyway" on the memory card. */
const OUTLINE_BTN_CLASS =
  'text-[11.5px] font-semibold text-primary bg-transparent border border-primary/45 rounded-lg px-3 py-1.5 cursor-pointer';

/** Emphasized "Always allow this model" button: the same outline plus a soft
 *  primary fill, marking it as the remember choice among the split actions. */
const ALWAYS_ALLOW_BTN_CLASS =
  'text-[11.5px] font-semibold text-primary bg-primary/[0.14] border border-primary/45 rounded-lg px-3 py-1.5 cursor-pointer';

/** Ghost secondary action button (no border, muted text). */
const GHOST_BTN_CLASS =
  'text-[11.5px] font-medium text-white/50 bg-transparent border-0 px-1 py-1.5 cursor-pointer';

/**
 * Renders a Minimal Line error callout inline in the chat thread.
 *
 * For `EngineStartFailed` the card shows a fixed human title and the full
 * backend detail verbatim in a wrapped, scrollable block (no per-error
 * translation or cleanup), plus a "Switch model" recovery action so a failed
 * load is never a dead end. For `InsufficientMemory` (issue #296) with
 * `insufficientMemoryInfo` present, the card shows a dedicated three-line
 * warning plus "Switch model" and "Load anyway" recovery actions; without
 * that info it falls back to the generic render below. Every other kind
 * splits the message on the first newline into title and subtitle, and the
 * subtitle renders an ollama pull command as an inline code element.
 */
export function ErrorCard({
  kind,
  message,
  onSwitchModel,
  onLoadAnyway,
  insufficientMemoryInfo,
}: ErrorCardProps) {
  /**
   * Renders the card's left accent bar in `color`. Factored so the freeze-band
   * body can tint it red to match its severity tag without duplicating the bar
   * markup or mutating `barColors`, which every other kind still reads.
   */
  const accentBar = (color: string) => (
    <div
      data-error-bar
      data-kind={kind}
      className="w-[2.5px] rounded-sm flex-shrink-0 self-stretch min-h-[36px]"
      style={{ background: color }}
    />
  );
  const bar = accentBar(barColors[kind]);

  if (kind === 'EngineStartFailed') {
    return (
      <div className="flex items-stretch gap-3 px-1 py-2 rounded-md bg-white/[0.025]">
        {bar}
        <div className="min-w-0">
          <p className="text-[12.5px] font-[590] text-white/[0.82] leading-snug tracking-[-0.01em]">
            {ENGINE_START_FAILED_TITLE}
          </p>
          {/* The raw engine error shown in full: wraps and scrolls inside the
              box so even a long blob path can never run off the window. No
              per-error translation. */}
          <p
            className="font-mono text-[10.5px] text-white/50 leading-relaxed mt-[7px] rounded-[7px] bg-white/[0.03] border border-white/[0.045] px-2.5 py-2"
            style={{
              whiteSpace: 'normal',
              overflowWrap: 'anywhere',
              wordBreak: 'break-word',
              maxHeight: '84px',
              overflow: 'auto',
            }}
          >
            {message}
          </p>
          {onSwitchModel && (
            <div className="flex gap-2 mt-[11px]">
              <button
                type="button"
                onClick={onSwitchModel}
                className="text-[11.5px] font-semibold text-primary bg-transparent border border-primary/45 rounded-lg px-3 py-1.5 cursor-pointer"
              >
                Switch model
              </button>
            </div>
          )}
        </div>
      </div>
    );
  }

  if (kind === 'InsufficientMemory' && insufficientMemoryInfo) {
    const { modelName, requiredBytes, availableBytes, canRemember } =
      insufficientMemoryInfo;
    // Freeze band: severity-first. The chip plus a single blunt note replace the
    // usual fit/estimate lines and the consequence copy, because at this ratio
    // the only useful message is how bad it is. No remember is possible here, so
    // "Always allow this model" is never offered.
    if (!canRemember) {
      return (
        <div className="flex items-stretch gap-3 px-1 py-2 rounded-md bg-white/[0.025]">
          {/* Red accent, not the shared amber: it must read as one severity
              signal with the tag rather than clashing with it. */}
          {accentBar(MEMORY_CRITICAL_RED)}
          <div className="min-w-0">
            <MemoryCriticalChip />
            <p className="text-[12.5px] font-[590] text-white/[0.82] leading-snug tracking-[-0.01em] mt-[7px]">
              {`Only ~${formatGb(availableBytes)} GB free. ${modelName} needs ~${formatGb(requiredBytes)} GB.`}
            </p>
            <p
              data-testid="memory-freeze-note"
              className="text-[11.5px] text-white/[0.38] leading-snug mt-0.5"
            >
              {MEMORY_FREEZE_NOTE}
            </p>
            {(onSwitchModel || onLoadAnyway) && (
              <div className="flex flex-wrap items-center gap-2 mt-[11px]">
                {onLoadAnyway && (
                  <button
                    type="button"
                    onClick={() => onLoadAnyway(false)}
                    className={OUTLINE_BTN_CLASS}
                  >
                    Load anyway
                  </button>
                )}
                {onSwitchModel && (
                  <button
                    type="button"
                    onClick={onSwitchModel}
                    className={GHOST_BTN_CLASS}
                  >
                    Switch model
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
      );
    }
    return (
      <div className="flex items-stretch gap-3 px-1 py-2 rounded-md bg-white/[0.025]">
        {bar}
        <div className="min-w-0">
          <p className="text-[12.5px] font-[590] text-white/[0.82] leading-snug tracking-[-0.01em]">
            {`${modelName} may not fit in memory right now.`}
          </p>
          <p className="text-[11.5px] text-white/[0.38] leading-snug mt-0.5">
            {`Estimated need: ~${formatGb(requiredBytes)} GB. Currently available: ~${formatGb(availableBytes)} GB.`}
          </p>
          <p className="text-[11.5px] text-white/[0.38] leading-snug mt-0.5">
            {INSUFFICIENT_MEMORY_CONSEQUENCE}
          </p>
          {(onSwitchModel || onLoadAnyway) && (
            <div className="flex flex-wrap items-center gap-2 mt-[11px]">
              {/* Mild band: split the force into "Load once" and the emphasized
                  "Always allow this model" (persists the per-model override),
                  with Switch model demoted to the ghost escape. */}
              {onLoadAnyway && (
                <>
                  <button
                    type="button"
                    onClick={() => onLoadAnyway(false)}
                    className={OUTLINE_BTN_CLASS}
                  >
                    Load once
                  </button>
                  <button
                    type="button"
                    onClick={() => onLoadAnyway(true)}
                    className={ALWAYS_ALLOW_BTN_CLASS}
                  >
                    Always allow this model
                  </button>
                </>
              )}
              {onSwitchModel && (
                <button
                  type="button"
                  onClick={onSwitchModel}
                  className={GHOST_BTN_CLASS}
                >
                  Switch model
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    );
  }

  const newlineIndex = message.indexOf('\n');
  const title = newlineIndex === -1 ? message : message.slice(0, newlineIndex);
  const subtitle = newlineIndex === -1 ? null : message.slice(newlineIndex + 1);

  const subtitleParts = subtitle ? renderSubtitle(subtitle) : null;

  return (
    <div className="flex items-stretch gap-3 px-1 py-2 rounded-md bg-white/[0.025]">
      {bar}
      <div>
        <p className="text-[12.5px] font-[590] text-white/[0.82] leading-snug tracking-[-0.01em]">
          {title}
        </p>
        {subtitleParts && (
          <p className="text-[11.5px] text-white/[0.38] leading-snug mt-0.5">
            {subtitleParts}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * Renders the subtitle string, wrapping `ollama pull <model>` in a <code> element.
 */
function renderSubtitle(text: string): React.ReactNode {
  const match = text.match(/^(.*?)(ollama pull \S+)(.*)$/);
  if (!match) return text;
  const [, before, command, after] = match;
  return (
    <>
      {before}
      <code className="font-mono text-[10.5px] bg-white/[0.07] text-white/50 px-[5px] py-px rounded">
        {command}
      </code>
      {after}
    </>
  );
}
