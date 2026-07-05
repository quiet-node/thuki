// Prints the pinned gate model's download URL, sha256, and cache filename as
// shell-eval-able lines, so the workflow reads them from the single source of
// truth (config.ts) instead of duplicating the pin in YAML.

import { GATE_MODEL, modelResolveUrl } from './config';

process.stdout.write(
  [
    `GATE_MODEL_URL=${modelResolveUrl(GATE_MODEL)}`,
    `GATE_MODEL_SHA256=${GATE_MODEL.sha256}`,
    `GATE_MODEL_FILE=${GATE_MODEL.file}`,
    '',
  ].join('\n'),
);
