// Fetches the pinned llama.cpp `llama-server` sidecar for the bundled
// inference engine. Runs before every dev/build (see package.json); the
// stamp file makes repeat runs an instant no-op. macOS arm64 only: other
// platforms (Ubuntu CI lint/test jobs) exit 0 without fetching anything.

import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  copyFile,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  realpath,
  rm,
  stat,
  writeFile,
} from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const LLAMA_CPP_TAG = 'b9590';
const ASSET = `llama-${LLAMA_CPP_TAG}-bin-macos-arm64.tar.gz`;
const ASSET_SHA256 =
  'b12cb8851ea60433e62522e43aa1dc9e820b4096b39d8c51e3cf7b1fba82102d';
const DEST = 'src-tauri/binaries';
const BIN = `${DEST}/llama-server-aarch64-apple-darwin`;
const STAMP = `${DEST}/.llama-cpp-version`;

const DOWNLOAD_URL = `https://github.com/ggml-org/llama.cpp/releases/download/${LLAMA_CPP_TAG}/${ASSET}`;
const STAMP_CONTENT = `${LLAMA_CPP_TAG} ${ASSET_SHA256}`;

const repoRoot = fileURLToPath(new URL('../', import.meta.url));
const destDir = resolve(repoRoot, DEST);
const binPath = resolve(repoRoot, BIN);
const stampPath = resolve(repoRoot, STAMP);

function fail(message: string): never {
  console.error(`ensure-llama-server: ${message}`);
  process.exit(1);
}

function run(command: string, args: string[]): string {
  const result = spawnSync(command, args, { encoding: 'utf8' });
  if (result.error) {
    fail(`${command} failed to start: ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(`${command} ${args.join(' ')} exited ${result.status}:\n${result.stderr}`);
  }
  return result.stdout;
}

async function exists(path: string): Promise<boolean> {
  return stat(path).then(
    () => true,
    () => false,
  );
}

// Parses `otool -L` output into the @rpath dylib names a Mach-O file links.
function rpathDeps(machoPath: string): string[] {
  const output = run('otool', ['-L', machoPath]);
  const deps: string[] = [];
  for (const line of output.split('\n')) {
    const match = /^\s+@rpath\/(lib[^ ]+\.dylib)/.exec(line);
    if (match) {
      deps.push(match[1]);
    }
  }
  return deps;
}

if (process.platform !== 'darwin' || process.arch !== 'arm64') {
  console.log(
    `ensure-llama-server: skipping on ${process.platform}/${process.arch} (sidecar is macOS arm64 only)`,
  );
  process.exit(0);
}

// Fast path: pinned version already installed.
if (await exists(binPath)) {
  const stamp = await readFile(stampPath, 'utf8').catch(() => '');
  if (stamp.trim() === STAMP_CONTENT) {
    process.exit(0);
  }
}

console.log(`ensure-llama-server: fetching llama.cpp ${LLAMA_CPP_TAG}...`);
const workDir = await mkdtemp(join(tmpdir(), 'thuki-llama-'));
try {
  // Download and verify against the pinned hash before touching anything.
  const response = await fetch(DOWNLOAD_URL);
  if (!response.ok) {
    fail(`download failed: HTTP ${response.status} for ${DOWNLOAD_URL}`);
  }
  const archive = Buffer.from(await response.arrayBuffer());
  const actualSha256 = createHash('sha256').update(archive).digest('hex');
  if (actualSha256 !== ASSET_SHA256) {
    fail(
      `sha256 mismatch for ${ASSET}\n  expected: ${ASSET_SHA256}\n  actual:   ${actualSha256}\nRefusing to install. The release asset may have been tampered with or the pin is stale.`,
    );
  }

  const archivePath = join(workDir, ASSET);
  await writeFile(archivePath, archive);
  run('tar', ['-xzf', archivePath, '-C', workDir]);

  const extractedDir = join(workDir, `llama-${LLAMA_CPP_TAG}`);
  const serverPath = join(extractedDir, 'llama-server');
  if (!(await exists(serverPath))) {
    fail(
      `archive layout unexpected: ${extractedDir}/llama-server not found after extraction`,
    );
  }

  // Index every dylib in the archive by name (recursively, in case the
  // layout ever moves them into a lib/ subdirectory).
  const dylibByName = new Map<string, string>();
  async function indexDylibs(dir: string): Promise<void> {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) {
        await indexDylibs(path);
      } else if (/^lib.+\.dylib$/.test(entry.name)) {
        dylibByName.set(entry.name, path);
      }
    }
  }
  await indexDylibs(extractedDir);

  // Walk the @rpath link closure starting from llama-server so we copy
  // exactly the dylibs it needs and skip other tools' impl dylibs.
  const needed = new Set<string>();
  const queue = rpathDeps(serverPath);
  while (queue.length > 0) {
    const name = queue.shift() as string;
    if (needed.has(name)) {
      continue;
    }
    const path = dylibByName.get(name);
    if (path === undefined) {
      fail(`llama-server links @rpath/${name} but the archive does not contain it`);
    }
    needed.add(name);
    queue.push(...rpathDeps(path));
  }

  await mkdir(destDir, { recursive: true });
  await copyFile(serverPath, binPath);
  const installedDylibs: string[] = [];
  for (const name of [...needed].sort()) {
    const target = join(destDir, name);
    // Dereference symlinks: versioned dylib names may be links to the
    // real file, and the bundle needs regular files.
    await copyFile(await realpath(dylibByName.get(name) as string), target);
    installedDylibs.push(target);
  }

  // At bundle time the sidecar lands in Contents/MacOS while the dylibs go
  // to Contents/Frameworks; in dev they sit next to the binary, which the
  // archive's existing @loader_path rpath already covers.
  const rpathResult = spawnSync(
    'install_name_tool',
    ['-add_rpath', '@loader_path/../Frameworks', binPath],
    { encoding: 'utf8' },
  );
  if (
    rpathResult.status !== 0 &&
    !rpathResult.stderr.includes('would duplicate path')
  ) {
    fail(`install_name_tool failed:\n${rpathResult.stderr}`);
  }

  // The rpath edit invalidates the ad-hoc linker signature; re-sign
  // everything we installed so macOS will execute it.
  for (const path of [binPath, ...installedDylibs]) {
    run('codesign', ['--force', '-s', '-', path]);
  }

  await writeFile(stampPath, `${STAMP_CONTENT}\n`, 'utf8');
  console.log(
    `ensure-llama-server: installed llama-server ${LLAMA_CPP_TAG} and ${installedDylibs.length} dylibs into ${DEST}`,
  );
} finally {
  await rm(workDir, { recursive: true, force: true });
}
