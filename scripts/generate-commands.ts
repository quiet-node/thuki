import { mkdir, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  renderCommandsMarkdown,
  renderSlashCommandPromptAppendix,
} from '../src/config/commandArtifacts';

const repoRoot = fileURLToPath(new URL('../', import.meta.url));
const outputs = [
  {
    path: resolve(repoRoot, 'docs/commands.md'),
    content: renderCommandsMarkdown(),
  },
  {
    path: resolve(
      repoRoot,
      'src-tauri/prompts/generated/slash_commands.txt',
    ),
    content: renderSlashCommandPromptAppendix(),
  },
];

await Promise.all(
  outputs.map(async ({ path, content }) => {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, content, 'utf8');
  }),
);
