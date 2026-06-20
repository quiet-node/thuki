import { describe, it, expect } from 'vitest';
import { downloadLine } from '../DownloadProgress';

const base = {
  progress: null,
  etaSeconds: null,
  combinedBytes: null,
  grandTotalBytes: null,
  speedBytesPerSec: null,
};

describe('downloadLine', () => {
  it('uses the unified combined/grand-total figure with ETA from etaSeconds', () => {
    const line = downloadLine({
      ...base,
      combinedBytes: 1.2e9,
      grandTotalBytes: 2.0e9,
      etaSeconds: 240,
    });
    expect(line).toEqual({ percent: 60, figures: '1.2 / 2.0 GB · ~4m' });
  });

  it('derives the unified ETA from the rolling speed when present', () => {
    const line = downloadLine({
      ...base,
      combinedBytes: 1.0e9,
      grandTotalBytes: 2.0e9,
      speedBytesPerSec: 1e8,
      // etaSeconds is ignored on the unified path when a speed is available.
      etaSeconds: 9999,
    });
    expect(line).toEqual({ percent: 50, figures: '1.0 / 2.0 GB · ~10s' });
  });

  it('falls back to per-file progress when no grand total is known', () => {
    const line = downloadLine({
      ...base,
      progress: { file: 'w.gguf', bytes: 2.5e9, totalBytes: 8.2e9 },
      etaSeconds: 300,
    });
    expect(line).toEqual({ percent: 30, figures: '2.5 / 8.2 GB · ~5m' });
  });

  it('clamps the unified percent to 100 and omits ETA when unmeasurable', () => {
    const line = downloadLine({
      ...base,
      combinedBytes: 2.1e9,
      grandTotalBytes: 2.0e9,
    });
    expect(line).toEqual({ percent: 100, figures: '2.1 / 2.0 GB' });
  });

  it('formats a multi-hour ETA on the per-file path', () => {
    const line = downloadLine({
      ...base,
      progress: { file: 'w.gguf', bytes: 1e9, totalBytes: 10e9 },
      etaSeconds: 7300,
    });
    expect(line).toEqual({ percent: 10, figures: '1.0 / 10.0 GB · ~2h 1m' });
  });

  it('returns 0% and no figures before any bytes are known', () => {
    expect(downloadLine(base)).toEqual({ percent: 0, figures: null });
  });

  it('returns 0% and no figures when the per-file total is zero', () => {
    const line = downloadLine({
      ...base,
      progress: { file: 'w.gguf', bytes: 10, totalBytes: 0 },
    });
    expect(line).toEqual({ percent: 0, figures: null });
  });
});
