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
  useMemo,
  useState,
  type ReactNode,
} from 'react';
import {
  useDownloadModel,
  type UseDownloadModel,
} from '../hooks/useDownloadModel';
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
   * tier + option, and kicks off the machine.
   */
  beginDownload: (tier: StarterTier, option: StarterOption) => void;
  /**
   * Resume an interrupted download: floors the bar at `partialBytes`, records
   * the tier + option, and restarts the machine.
   */
  resumeDownload: (
    tier: StarterTier,
    option: StarterOption,
    partialBytes: number,
  ) => void;
}

const DownloadContext = createContext<DownloadContextValue | null>(null);

export function DownloadProvider({ children }: { children: ReactNode }) {
  const download = useDownloadModel();
  const [downloadingTier, setDownloadingTier] = useState<StarterTier | null>(
    null,
  );
  const [resumeSeedBytes, setResumeSeedBytes] = useState<number | null>(null);
  const [activeOption, setActiveOption] = useState<StarterOption | null>(null);

  const { start, resume } = download;

  const beginDownload = useCallback(
    (tier: StarterTier, option: StarterOption) => {
      setResumeSeedBytes(null);
      setDownloadingTier(tier);
      setActiveOption(option);
      void start(tier);
    },
    [start],
  );

  const resumeDownload = useCallback(
    (tier: StarterTier, option: StarterOption, partialBytes: number) => {
      setResumeSeedBytes(partialBytes);
      setDownloadingTier(tier);
      setActiveOption(option);
      void resume(tier);
    },
    [resume],
  );

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
    }),
    [
      download,
      downloadingTier,
      resumeSeedBytes,
      activeOption,
      grandTotalBytes,
      beginDownload,
      resumeDownload,
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
