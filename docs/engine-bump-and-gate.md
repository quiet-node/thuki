# Keeping the bundled engine current

Two pieces keep Thuki's bundled llama.cpp engine fresh without shipping a regression, and both follow how the field actually does it (Ollama gates a `LLAMA_CPP_VERSION` change with a build; Jan/cortex gate a submodule bump with a build + real-model E2E smoke):

1. **Engine Regression Gate** — builds the engine and proves it loads a real model and runs real inference correctly. Runs on every PR; the hard gate on bump PRs.
2. **Renovate** — opens scheduled, review-gated PRs that advance the pin. Never auto-merged.

The pin is two constants in `scripts/ensure-llama-server.ts`: `LLAMA_CPP_TAG` and `LLAMA_CPP_COMMIT`. The commit is the supply-chain anchor (the build refuses to proceed unless the tag resolves to it). See [release-process.md](./release-process.md#bumping-the-pinned-llamacpp-version) for the pin mechanics.

## Engine Regression Gate

`.github/workflows/engine-regression-gate.yml`, on every PR and pushes to `main`:

1. Builds the shipped engine through the real `bun run engine:ensure` — so a changed dylib closure, a broken macOS-floor audit, or a signing failure trips here, not in a user's hands.
2. Verifies `codesign -vv` on the `llama-server` binary and every bundled dylib.
3. Downloads the pinned gate model (verified by sha256, cached) and drives it through `llama-server`'s `/v1/chat/completions` endpoint — Thuki's real runtime path, so a broken chat template is caught — with greedy decoding (per-request `samplers: ["top_k"]`, `top_k: 1`, `seed: 0`, penalties off).
4. Asserts the answers are **semantically** correct, not byte-identical to a frozen golden (greedy output is only reproducible on the same build + hardware, so a golden would false-fail across a bump while real breakage — crash, garbage, broken template, unsupported architecture — still trips).
5. Reads the generation throughput from llama-server's own timings and fails on a **catastrophic-throughput floor** (a silent CPU fallback). The exact tok/s is printed in the job summary; subtler performance regressions are caught by the human review on the (rare) bump PR — a deliberate choice not to chase a noisy absolute perf baseline on shared CI runners.

The decision logic (`scripts/engine-gate/assertions.ts`, `report.ts`) is pure and unit-tested; the workflow shell collects the raw signals and `decide.ts` judges them.

### Gate model

`Qwen/Qwen2.5-1.5B-Instruct` Q4_K_M, pinned in `scripts/engine-gate/config.ts` by exact Hugging Face revision + file sha256. Apache-2.0, mainstream `qwen2` architecture, a well-defined chat template, and strong factual correctness at 1.5B so the assertions do not false-fail on a borderline answer.

### Why this and not a KL-divergence / perplexity / perf-ratio gate

Because no inspectable project in this space runs one. Ollama's llama.cpp-update workflow builds the runtime; correctness comes from its normal integration tests. Jan/cortex's submodule-bump quality gate builds a matrix and runs a real-model E2E smoke. Neither compares logit distributions or gates a throughput ratio against the previous build. Build + load-a-real-model + generate + assert is the field standard, and matching it (rather than building a bespoke numeric gate that needs threshold calibration and babysitting) is the right reliability/maintainability trade.

The one place the field is demonstrably weak is performance: Ollama shipped a [~56% regression](https://github.com/ollama/ollama/issues/15601) from a bump because it had no perf gate. The throughput floor plus the printed tok/s on the human-reviewed bump PR is the deliberately cheap answer to that; if it proves insufficient, the right next step is a simple recorded-throughput comparison, not a KL apparatus.

## Bump via Renovate

`.github/renovate.json` defines a scoped custom manager that tracks `ggml-org/llama.cpp` tags and bumps `LLAMA_CPP_TAG` (+ the pinned `LLAMA_CPP_COMMIT` digest) in `scripts/ensure-llama-server.ts`:

- **Scoped**: `enabledManagers: ["custom.regex"]` so Renovate only manages this one pin and does not fight the repo's existing Dependabot config.
- **Cadence + cooldown**: `schedule` is weekly; `minimumReleaseAge: "3 days"` is the cooldown that lets a broken same-day upstream build surface before Thuki rolls to it.
- **Versioning**: `regex:^b(?<major>\\d+)$` orders llama.cpp's `bNNNN` tags correctly.
- **Never auto-merged**: the PR is human-reviewed, and the Engine Regression Gate is the hard check on it.

Renovate opens the bump PR; the gate validates it. The **cache keys need no updating** — every workflow keys the sidecar cache on `hashFiles('scripts/ensure-llama-server.ts')`, so changing the pin invalidates the cache automatically.

### Dylib closure changes (rare)

When an upstream bump adds or drops a dylib, `engine:ensure` aborts with the exact difference (`needed but not listed: …` / `listed but not in the closure: …`). On such a bump the gate fails loudly; check out the Renovate branch, run `bun run engine:ensure`, update `bundle.macOS.frameworks` in `src-tauri/tauri.conf.json` to match, and push. This is intentionally a human step: it is a change to the signed bundle's contents and happens rarely.

## Setup

1. **Enable Renovate** on the repo (the Mend Renovate GitHub App, or self-hosted). No secrets needed for the custom manager.
2. **Branch protection**: make `Engine loads + inference` a required status check so a failing gate blocks merge.
3. First bump PR: review the upstream diff + the gate result, then merge. That's the whole loop.
