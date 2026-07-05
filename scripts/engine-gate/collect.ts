// Starts the freshly built llama-server on the pinned gate model, sends each
// correctness prompt through the OpenAI-compatible /v1/chat/completions endpoint
// (Thuki's real runtime path, so a broken chat template is exercised), and writes
// the responses plus the observed generation throughput to GATE_RESULT for
// decide.ts to judge. Pure I/O; the judging lives in the unit-tested core.

import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';

import { SAMPLING } from './config';
import { CORRECTNESS_PROMPTS } from './prompts';

const BINARY = 'src-tauri/binaries/llama-server-aarch64-apple-darwin';
const PORT = 8181;
const BASE = `http://127.0.0.1:${PORT}`;
const HEALTH_TIMEOUT_MS = 120_000;
const GEN_TIMEOUT_MS = 120_000;

function required(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`engine-gate collect: missing required env ${name}`);
  }
  return value;
}

async function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForHealth(deadline: number): Promise<void> {
  while (Date.now() < deadline) {
    try {
      if ((await fetch(`${BASE}/health`)).ok) {
        return;
      }
    } catch {
      // server not up yet
    }
    await sleep(500);
  }
  throw new Error('engine-gate collect: llama-server did not become healthy in time');
}

interface ChatResult {
  content: string;
  tps: number;
}

async function chat(prompt: string): Promise<ChatResult> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), GEN_TIMEOUT_MS);
  try {
    const res = await fetch(`${BASE}/v1/chat/completions`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      signal: controller.signal,
      body: JSON.stringify({
        model: 'gate',
        messages: [{ role: 'user', content: prompt }],
        samplers: [SAMPLING.samplers],
        top_k: SAMPLING.topK,
        seed: SAMPLING.seed,
        repeat_penalty: SAMPLING.repeatPenalty,
        max_tokens: SAMPLING.nPredict,
        stream: false,
      }),
    });
    if (!res.ok) {
      throw new Error(`chat request failed: HTTP ${res.status}`);
    }
    const body = (await res.json()) as {
      choices: { message: { content: string } }[];
      timings?: { predicted_per_second?: number };
    };
    return {
      content: body.choices[0].message.content,
      tps: body.timings?.predicted_per_second ?? 0,
    };
  } finally {
    clearTimeout(timer);
  }
}

async function main(): Promise<void> {
  const modelPath = required('GATE_MODEL_PATH');
  const outPath = required('GATE_RESULT');

  const server = spawn(
    BINARY,
    [
      '--model',
      modelPath,
      '--port',
      String(PORT),
      '--host',
      '127.0.0.1',
      '--parallel',
      String(SAMPLING.parallel),
      '--ctx-size',
      '2048',
      '--no-warmup',
    ],
    { stdio: 'inherit' },
  );

  try {
    await waitForHealth(Date.now() + HEALTH_TIMEOUT_MS);
    const responses: Record<string, string> = {};
    let tps = 0;
    for (const assertion of CORRECTNESS_PROMPTS) {
      const result = await chat(assertion.prompt);
      responses[assertion.id] = result.content;
      tps = Math.max(tps, result.tps);
    }
    writeFileSync(outPath, JSON.stringify({ responses, tps }, null, 2));
    process.stdout.write(`engine-gate collect: wrote ${outPath} (peak ${tps.toFixed(1)} tok/s)\n`);
  } finally {
    server.kill('SIGTERM');
  }
}

main().catch((err) => {
  process.stderr.write(`${String(err)}\n`);
  process.exitCode = 1;
});
