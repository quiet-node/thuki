// Configuration for the engine regression gate: the pinned test model, the
// deterministic decoding settings, and the throughput floor. The model pin
// carries the same provenance discipline as the engine itself — an exact Hugging
// Face repo revision plus the file's sha256 — so it can never silently change.

export interface ModelPin {
  repo: string;
  /** Immutable Hugging Face commit sha the file is fetched from. */
  revision: string;
  file: string;
  /** sha256 of the GGUF, verified after download. */
  sha256: string;
  sizeBytes: number;
}

// Qwen2.5-1.5B-Instruct, Q4_K_M. Apache-2.0, mainstream qwen2 architecture, a
// well-defined chat template, and strong factual correctness at 1.5B so the
// semantic assertions do not false-fail on a borderline answer.
export const GATE_MODEL: ModelPin = {
  repo: 'Qwen/Qwen2.5-1.5B-Instruct-GGUF',
  revision: '91cad51170dc346986eccefdc2dd33a9da36ead9',
  file: 'qwen2.5-1.5b-instruct-q4_k_m.gguf',
  // When this sha256 changes, also bump the `gate-model-<sha8>` cache key in
  // .github/workflows/engine-regression-gate.yml, or the gate restores the stale
  // model from cache.
  sha256: '6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e',
  sizeBytes: 1117320736,
};

// Resolves the download URL for a pinned file at an immutable revision. Pinning
// by revision (not a branch) makes the bytes immutable, which is what lets the
// sha256 check mean something.
export function hfResolveUrl(repo: string, revision: string, file: string): string {
  return `https://huggingface.co/${repo}/resolve/${revision}/${file}`;
}

export function modelResolveUrl(pin: ModelPin): string {
  return hfResolveUrl(pin.repo, pin.revision, pin.file);
}

// Deterministic decoding. Since llama.cpp PR #9897, temp=0 no longer guarantees
// greedy; a single top_k=1 sampler with penalties disabled is the reliable way to
// take the argmax token every step. A single slot with no continuous batching
// removes cross-request nondeterminism.
export const SAMPLING = {
  samplers: 'top_k',
  topK: 1,
  seed: 0,
  repeatPenalty: 1.0,
  nPredict: 64,
  parallel: 1,
};

// Catastrophic-throughput floor (tokens/sec), measured from llama-server's own
// timings during the smoke. A healthy Metal build of this model on Apple Silicon
// runs many multiples of this; the floor exists only to catch gross breakage such
// as a silent CPU fallback. Subtler performance regressions are surfaced in the
// gate summary and caught by the mandatory human review on engine-bump PRs — a
// deliberate choice not to chase a noisy absolute perf baseline on shared CI
// runners (see docs/engine-bump-and-gate.md).
export const FLOOR_TPS = 10;
