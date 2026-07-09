/**
 * App-root download context.
 *
 * Lifts the single starter-model download machine above the onboarding
 * stage split so a download survives `ModelCheckStep` unmounting when the
 * user taps "Continue" mid-download. The picker, the onboarding intro, and
 * the ask bar all read one live download from here.
 *
 * It wraps `useDownloadModel` (engine handoff off: the engine starts lazily
 * on the first chat, so `AllDone` is terminal at `ready`) and adds the bits
 * the picker used to own locally: which tier is downloading, the resume-seed
 * floor, the active option, and the card's grand total (weights + vision
 * companion) the ambient strip needs to render percent.
 */

import {
  createContext,
  use,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { invoke } from '@tauri-apps/api/core';
import {
  isDownloadInFlight,
  useDownloadModel,
  type UseDownloadModel,
} from '../hooks/useDownloadModel';
import { useConfig } from './ConfigContext';
import { useStarterOptions } from '../components/StarterPicker';
import type { StarterOption, StarterTier } from '../types/starter';

export interface DownloadContextValue extends UseDownloadModel {
  /** Tier whose download is in flight; null when idle. */
  downloadingTier: StarterTier | null;
  /**
   * Bytes already on disk for a resumed download, flooring the bar at the
   * paused position until the first real event lands. Null for a fresh
   * (non-resume) download.
   */
  resumeSeedBytes: number | null;
  /** The option being downloaded; carries the grand total the strip needs. */
  activeOption: StarterOption | null;
  /**
   * The active option's full on-disk cost (weights + vision companion), or
   * null when no download is active.
   */
  grandTotalBytes: number | null;
  /**
   * Start a fresh download for a tier: clears the resume seed, records the
   * tier + option, and kicks off the machine. `userInitiated` (default true)
   * threads to the download command's safe-mode gate (issue #296); the
   * launch-time auto-resume effect below is the one caller that passes false.
   */
  beginDownload: (
    tier: StarterTier,
    option: StarterOption,
    userInitiated?: boolean,
  ) => void;
  /**
   * Resume an interrupted download: floors the bar at `partialBytes`, records
   * the tier + option, and restarts the machine. Same `userInitiated`
   * contract as {@link beginDownload}.
   */
  resumeDownload: (
    tier: StarterTier,
    option: StarterOption,
    partialBytes: number,
    userInitiated?: boolean,
  ) => void;
  /** True while a started download has been paused (cancelled, partial kept). */
  isPaused: boolean;
  /**
   * True the instant Pause is clicked, until the cancel lands (the download is
   * still in flight). Drives the transitional "Pausing…" strip so the click
   * has immediate feedback before `isPaused` commits at idle.
   */
  isPausing: boolean;
  /** Bytes downloaded at the moment of pause, for the paused strip's percent. */
  pausedBytes: number;
  /** Pause the in-flight download: cancel it; the partial stays on disk. */
  pauseDownload: () => void;
  /** Resume a paused download from where it stopped. */
  resumeFromPause: () => void;
  /**
   * Discard the paused download: delete its partial bytes (weights + vision
   * companion) from disk and clear the ambient strip. Unlike pause there is no
   * resume after this; the model-picker chip is the way back to start another
   * download. Lets a user abandon a first-model download they no longer want
   * (e.g. after picking a smaller model in Settings) instead of being held to
   * a Resume-only loop.
   */
  discardDownload: () => void;
}

const DownloadContext = createContext<DownloadContextValue | null>(null);

export function DownloadProvider({ children }: { children: ReactNode }) {
  const download = useDownloadModel();
  const [downloadingTier, setDownloadingTier] = useState<StarterTier | null>(
    null,
  );
  const [resumeSeedBytes, setResumeSeedBytes] = useState<number | null>(null);
  const [activeOption, setActiveOption] = useState<StarterOption | null>(null);
  const [pauseRequested, setPauseRequested] = useState(false);
  const [localPausedBytes, setLocalPausedBytes] = useState(0);

  const { start, resume, cancel, discard, combinedBytes } = download;
  const downloadPhase = download.state.phase;

  // The on-disk truth for the active model: its partial size and whether it is
  // installed. `useStarterOptions` re-reads this on every `models-changed`
  // broadcast (which a pause or discard from ANY window now fires), so this
  // stays current cross-window. Matched by the weights sha so it tracks exactly
  // the model the strip is showing.
  const { options: starterOptions } = useStarterOptions();
  const activeStarter =
    activeOption !== null && starterOptions !== null
      ? starterOptions.find(
          (o) => o.starter.sha256 === activeOption.starter.sha256,
        )
      : undefined;
  const activePartialBytes = activeStarter?.partial_bytes ?? null;
  const activeInstalled = activeStarter?.installed ?? false;

  // Paused is derived from durable truth, not a local flag: the machine is idle
  // (not actively streaming) and an interrupted partial exists on disk for the
  // active model that is not yet installed. This makes a pause from ANY window
  // show here, fixing the case where pausing from Settings used to make the
  // ambient strip vanish. The `pauseRequested` short-circuit keeps a pause
  // initiated HERE smooth: it commits the instant the cancel reaches idle,
  // before the cross-window partial re-read lands.
  const idle = downloadPhase === 'idle';
  const isPaused =
    (pauseRequested && idle) ||
    (idle && activePartialBytes !== null && !activeInstalled);
  // Percent source: the snapshot at pause time for a local pause (instant), the
  // on-disk partial for a pause observed from another window.
  const pausedBytes = pauseRequested
    ? localPausedBytes
    : (activePartialBytes ?? 0);
  // Transitional: the cancel is requested but the download is still winding
  // down. The strip shows "Pausing…" here so the Pause click is never silent.
  const isPausing = pauseRequested && isDownloadInFlight(downloadPhase);

  // A pause cancels the backend download task, so the slot is free and only the
  // frontend knows a download is paused. Report it so the quit warning fires
  // for a paused (or pausing) download too, not only an actively-streaming one.
  const pausedForQuitWarning = isPaused || isPausing;
  useEffect(() => {
    void invoke('set_download_paused', { paused: pausedForQuitWarning });
  }, [pausedForQuitWarning]);

  const beginDownload = useCallback(
    (tier: StarterTier, option: StarterOption, userInitiated = true) => {
      setResumeSeedBytes(null);
      setDownloadingTier(tier);
      setActiveOption(option);
      setPauseRequested(false);
      void start(tier, userInitiated);
    },
    [start],
  );

  const resumeDownload = useCallback(
    (
      tier: StarterTier,
      option: StarterOption,
      partialBytes: number,
      userInitiated = true,
    ) => {
      setResumeSeedBytes(partialBytes);
      setDownloadingTier(tier);
      setActiveOption(option);
      setPauseRequested(false);
      void resume(tier, userInitiated);
    },
    [resume],
  );

  // On launch, recover an interrupted built-in download: if the engine is the
  // active provider and a starter has a partial on disk but none is installed,
  // restart it in the background so the ambient strip is the recovery surface.
  // The relaunch no longer bounces the user back to the picker, so this is what
  // keeps them from being stranded with no model. Fires once: the ref guards
  // against the StrictMode double-invoke and any later provider re-render.
  const activeProviderKind = useConfig().inference.activeProviderKind;
  const autoResumedRef = useRef(false);
  useEffect(() => {
    if (autoResumedRef.current) return;
    autoResumedRef.current = true;
    if (activeProviderKind !== 'builtin') return;
    void (async () => {
      // The model_check picker owns the resume decision (its own Resume /
      // Discard choice), so only act once the user is past it: the intro tour
      // or the ask bar.
      const stage = await invoke<string>('onboarding_stage');
      if (stage !== 'intro' && stage !== 'complete') return;
      const options = await invoke<StarterOption[]>('get_starter_options');
      const partial = options.find((o) => o.partial_bytes !== null);
      if (options.some((o) => o.installed) || partial === undefined) return;
      // A cold-restart resume re-hashes the on-disk prefix and appends a Range
      // body, but that path fails verification against the live CDN every time,
      // so it would only ever re-download after a scary "did not verify" error.
      // Discard the partial(s) and download fresh instead: same bytes, no error.
      await discard(partial.starter.sha256);
      if (partial.starter.mmproj_sha256 !== null) {
        await discard(partial.starter.mmproj_sha256);
      }
      // Not a user click: a post-crash safe-mode session must refuse this
      // auto-restart (issue #296), not silently resume a download that may
      // have been implicated in the crash.
      beginDownload(partial.starter.tier, partial, false);
    })();
  }, [activeProviderKind, discard, beginDownload]);

  const pauseDownload = useCallback(() => {
    // Remember how far we got so the paused strip can show the percent, then
    // cancel the run (the backend keeps the partial on disk for resume). The
    // pause only *shows* once `downloadPhase` reaches idle (see `isPaused`).
    setLocalPausedBytes(combinedBytes ?? 0);
    setPauseRequested(true);
    void cancel();
  }, [combinedBytes, cancel]);

  const resumeFromPause = useCallback(() => {
    // Only reachable from the paused strip, which renders only when a download
    // was started, so the active option is always set here. resumeDownload
    // clears pauseRequested.
    resumeDownload(activeOption!.starter.tier, activeOption!, pausedBytes);
  }, [activeOption, pausedBytes, resumeDownload]);

  const discardDownload = useCallback(() => {
    // Only reachable from the paused strip, so a download was started and the
    // active option is set. The run is already cancelled (paused == idle), so
    // deleting the partial bytes and clearing the strip's state is all that
    // remains: clearing pauseRequested drops `isPaused`, and with the phase
    // already idle the ambient strip renders nothing.
    const { starter } = activeOption!;
    setPauseRequested(false);
    setDownloadingTier(null);
    setActiveOption(null);
    setResumeSeedBytes(null);
    setLocalPausedBytes(0);
    void (async () => {
      await discard(starter.sha256);
      if (starter.mmproj_sha256 !== null) {
        await discard(starter.mmproj_sha256);
      }
    })();
  }, [activeOption, discard]);

  const grandTotalBytes =
    activeOption === null
      ? null
      : activeOption.starter.size_bytes + activeOption.starter.mmproj_bytes;

  const value = useMemo<DownloadContextValue>(
    () => ({
      ...download,
      downloadingTier,
      resumeSeedBytes,
      activeOption,
      grandTotalBytes,
      beginDownload,
      resumeDownload,
      isPaused,
      isPausing,
      pausedBytes,
      pauseDownload,
      resumeFromPause,
      discardDownload,
    }),
    [
      download,
      downloadingTier,
      resumeSeedBytes,
      activeOption,
      grandTotalBytes,
      beginDownload,
      resumeDownload,
      isPaused,
      isPausing,
      pausedBytes,
      pauseDownload,
      resumeFromPause,
      discardDownload,
    ],
  );

  return <DownloadContext value={value}>{children}</DownloadContext>;
}

/**
 * Returns the app-root download machine. Throws when no `DownloadProvider`
 * wraps the caller: unlike config, there is no sensible static fallback for
 * a live download, so a missing provider is a wiring bug, not a test
 * convenience.
 */
export function useDownloadCtx(): DownloadContextValue {
  const value = use(DownloadContext);
  if (value === null) {
    throw new Error('useDownloadCtx must be used within a DownloadProvider');
  }
  return value;
}
