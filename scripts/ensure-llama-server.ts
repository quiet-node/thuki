// Builds the pinned llama.cpp `llama-server` sidecar from source for the bundled
// inference engine. Runs before every dev/build (see package.json); the stamp
// file makes repeat runs an instant no-op. macOS arm64 only: other platforms
// (Ubuntu CI lint/test jobs) exit 0 without building anything.
//
// Why build instead of downloading ggml-org's prebuilt: their official arm64
// release binary is built on a macOS-26 runner with no deployment-target floor,
// so it hard-imports a macOS-15 Metal symbol (MTLResidencySetDescriptor) and
// fails at dyld load on older macOS with "Symbol not found ... Metal.framework".
// Building with our MACOS_DEPLOYMENT_TARGET turns that into a weak import (used
// on macOS 15+, skipped below), so the engine runs on our floor and up while we
// track the latest llama.cpp. The freshly built sidecar is audited compatible
// (verifyMacosCompatible) before anything is installed.

import { spawnSync } from 'node:child_process';
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
import { cpus, tmpdir, totalmem } from 'node:os';
import { basename, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

// The pin: a llama.cpp release tag and its exact commit SHA. Pinning the commit
// (not just the tag) is the supply-chain anchor: the clone is verified to land
// on this commit, so a moved or forged tag is rejected before it is built. Bump
// both together; see "Bumping the pinned llama.cpp version" in
// docs/release-process.md.
const LLAMA_CPP_TAG = 'b9946';
const LLAMA_CPP_COMMIT = 'fb30ba9a6c5b4674174d06aed14794832ab33278';
const LLAMA_CPP_REPO = 'https://github.com/ggml-org/llama.cpp.git';

// Minimum macOS Thuki supports, and the engine's build deployment target. This
// is the single source of truth: the build flag, the install-time audit, and
// the stamp all derive from it, so they cannot drift. 13.4 matches LM Studio's
// llama.cpp floor and is more inclusive than Ollama (14) and Jan (13.6); keep it
// in sync with `minimumSystemVersion` in src-tauri/tauri.conf.json.
const MACOS_DEPLOYMENT_TARGET = '13.4';

// CMake flags that make the build macOS 13.4 compatible and bundle-shaped.
// CMAKE_OSX_DEPLOYMENT_TARGET is the load-bearing one (weak Metal import); the
// rest keep the output relocatable, dependency-clean, and identical in shape to
// what the bundle expects. See the file header for the why.
const CMAKE_FLAGS = [
  '-DCMAKE_BUILD_TYPE=Release',
  '-DCMAKE_OSX_ARCHITECTURES=arm64',
  `-DCMAKE_OSX_DEPLOYMENT_TARGET=${MACOS_DEPLOYMENT_TARGET}`,
  // No homebrew OpenSSL: it is absent on user Macs and is itself built for a
  // newer macOS, which would re-raise the deployment floor.
  '-DCMAKE_DISABLE_FIND_PACKAGE_OpenSSL=ON',
  // Relocatable: find sibling dylibs via @loader_path, never an absolute build path.
  '-DCMAKE_BUILD_WITH_INSTALL_RPATH=ON',
  '-DCMAKE_INSTALL_RPATH=@loader_path',
  '-DBUILD_SHARED_LIBS=ON',
  '-DGGML_METAL=ON',
  // Embed the Metal shaders in the dylib: no extra bundled file, no runtime compile.
  '-DGGML_METAL_EMBED_LIBRARY=ON',
  '-DGGML_METAL_USE_BF16=ON',
  '-DGGML_RPC=ON',
  '-DLLAMA_BUILD_SERVER=ON',
  '-DLLAMA_BUILD_TOOLS=ON',
  '-DLLAMA_BUILD_TESTS=OFF',
  '-DLLAMA_BUILD_EXAMPLES=OFF',
];

const DEST = 'src-tauri/binaries';
const BIN = `${DEST}/llama-server-aarch64-apple-darwin`;
const STAMP = `${DEST}/.llama-cpp-version`;
// The deployment target is part of the stamp: changing the supported macOS floor
// must force a rebuild even when the tag and commit are unchanged.
const STAMP_CONTENT = `${LLAMA_CPP_TAG} ${LLAMA_CPP_COMMIT} macos${MACOS_DEPLOYMENT_TARGET}`;

const repoRoot = fileURLToPath(new URL('../', import.meta.url));
const destDir = resolve(repoRoot, DEST);
const binPath = resolve(repoRoot, BIN);
const stampPath = resolve(repoRoot, STAMP);

function fail(message: string): never {
  console.error(`ensure-llama-server: ${message}`);
  process.exit(1);
}

// Runs a command and returns its stdout. For short, output-bearing tools
// (git rev-parse, otool, nm, lipo). Not for the build itself: spawnSync buffers
// stdout and a full cmake build overflows the default 1 MB cap.
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

// Runs a command streaming its output straight to the terminal. For the clone
// and build, whose output is large (overflows a captured buffer) and whose
// progress is worth seeing during the multi-minute build.
function runStreaming(command: string, args: string[]): void {
  const result = spawnSync(command, args, { stdio: 'inherit' });
  if (result.error) {
    fail(`${command} failed to start: ${result.error.message}`);
  }
  if (result.status !== 0) {
    fail(`${command} ${args.join(' ')} exited ${result.status}`);
  }
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

// Indexes every dylib under `dir` by name (recursively, in case the layout ever
// moves them into a lib/ subdirectory).
async function indexDylibs(dir: string, into: Map<string, string>): Promise<void> {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      await indexDylibs(path, into);
    } else if (/^lib.+\.dylib$/.test(entry.name)) {
      into.set(entry.name, path);
    }
  }
}

// Walks the @rpath link closure starting from llama-server so we know exactly
// which dylibs it needs (and skip other tools' impl dylibs). `source` names
// where the dylibs were expected, for the failure message.
function walkClosure(
  rootPath: string,
  dylibByName: Map<string, string>,
  source: string,
): Set<string> {
  const needed = new Set<string>();
  const queue = rpathDeps(rootPath);
  while (queue.length > 0) {
    const name = queue.shift() as string;
    if (needed.has(name)) {
      continue;
    }
    const path = dylibByName.get(name);
    if (path === undefined) {
      fail(`llama-server links @rpath/${name} but ${source} does not contain it`);
    }
    needed.add(name);
    queue.push(...rpathDeps(path));
  }
  return needed;
}

// Drift guard: the computed dylib closure must exactly match the hand-pinned
// bundle.macOS.frameworks list in tauri.conf.json. Without this, a pin bump that
// adds or renames a dylib would install it into binaries/ while the bundle
// silently omits it, and the breakage would only surface in the shipped .app.
async function verifyFrameworksList(needed: Set<string>): Promise<void> {
  const confRelPath = 'src-tauri/tauri.conf.json';
  const confPath = resolve(repoRoot, confRelPath);
  let frameworks: unknown;
  try {
    frameworks = JSON.parse(await readFile(confPath, 'utf8')).bundle?.macOS?.frameworks;
  } catch (error) {
    fail(`failed to read ${confRelPath}: ${(error as Error).message}`);
  }
  if (!Array.isArray(frameworks)) {
    fail(`bundle.macOS.frameworks is missing from ${confRelPath}`);
  }
  const pinned = new Set(frameworks.map((entry) => basename(String(entry))));
  const missing = [...needed].filter((name) => !pinned.has(name)).sort();
  const extra = [...pinned].filter((name) => !needed.has(name)).sort();
  if (missing.length > 0 || extra.length > 0) {
    const lines = [
      `dylib closure does not match bundle.macOS.frameworks in ${confRelPath}`,
    ];
    if (missing.length > 0) {
      lines.push(`  needed by llama-server but not listed: ${missing.join(', ')}`);
    }
    if (extra.length > 0) {
      lines.push(`  listed but not in the closure: ${extra.join(', ')}`);
    }
    lines.push(`Update the frameworks list in ${confRelPath} to match the closure.`);
    fail(lines.join('\n'));
  }
}

/**
 * Outcome of a build-tool probe. `unknown` means the probe itself could not
 * produce an answer (spawn threw, failed at the OS level for an unrelated
 * reason, or returned no exit status).
 */
type ProbeResult = 'present' | 'missing' | 'unknown';

/**
 * Probes a build tool by running a cheap version or lookup command.
 *
 * Every failure mode of the spawn is handled here: a probe must never throw out
 * of preflight. Only a definitive answer is reported as `missing`; anything
 * ambiguous is `unknown` so preflight stays out of the way and the existing
 * build path fails as it does today. A permissive probe that misses a missing
 * tool is an annoyance; a strict probe that trips on a working machine breaks
 * every dev build and every release.
 *
 * @param command Tool to probe, resolved through `$PATH`.
 * @param args Arguments for the probe invocation, kept cheap and side effect free.
 * @returns Whether the tool is definitively present, definitively missing, or unknown.
 */
function probeTool(command: string, args: string[]): ProbeResult {
  let result: ReturnType<typeof spawnSync>;
  try {
    result = spawnSync(command, args, { stdio: 'ignore' });
  } catch {
    return 'unknown';
  }
  if (result.error) {
    const code = (result.error as NodeJS.ErrnoException).code;
    // ENOENT (not on PATH) and EACCES (present but not executable) are the two
    // definitive answers; any other spawn error says nothing about the tool.
    return code === 'ENOENT' || code === 'EACCES' ? 'missing' : 'unknown';
  }
  if (typeof result.status !== 'number') {
    return 'unknown';
  }
  return result.status === 0 ? 'present' : 'missing';
}

/**
 * Checks the build prerequisites before any build work starts, and reports all
 * of them at once so a contributor does not install one tool, re-run, and
 * discover the next.
 *
 * Exits the process with an actionable message when a prerequisite is
 * definitively missing; returns normally otherwise.
 */
function preflightBuildTools(): void {
  const problems: string[] = [];

  if (probeTool('cmake', ['--version']) === 'missing') {
    problems.push(
      '  cmake is not installed. Install it with: brew install cmake',
      '    (see the prerequisites section of CONTRIBUTING.md)',
    );
  }

  // The metal compiler probe is the source of truth: if the compiler is already
  // there, the build needs nothing from Xcode.app and xcodebuild is irrelevant.
  // xcodebuild only matters on the one path that uses it, the toolchain
  // download in ensureMetalCompiler. `xcodebuild -version` is the same gate that
  // download hits: it exits nonzero when the active developer directory is a
  // Command Line Tools instance rather than Xcode.app.
  if (
    probeTool('xcrun', ['-sdk', 'macosx', '-f', 'metal']) === 'missing' &&
    probeTool('xcodebuild', ['-version']) === 'missing'
  ) {
    problems.push(
      '  the Metal shader compiler is missing and xcodebuild cannot download it,',
      '    because the active developer directory is a Command Line Tools instance.',
      '    Install Xcode from the App Store, then run:',
      '      sudo xcode-select -s /Applications/Xcode.app',
      '    (see the prerequisites section of CONTRIBUTING.md)',
    );
  }

  if (problems.length > 0) {
    fail(['missing build prerequisites:', ...problems].join('\n'));
  }
}

// Ensures Apple's Metal shader compiler is available. GGML_METAL_EMBED_LIBRARY
// needs it to compile the shaders into the dylib at build time. It ships with
// Xcode <= 16; Xcode 26 split it into a separately downloadable component. The
// download is Apple-signed and idempotent (a no-op when already installed).
function ensureMetalCompiler(): void {
  if (spawnSync('xcrun', ['-sdk', 'macosx', '-f', 'metal']).status === 0) {
    return;
  }
  console.log('ensure-llama-server: Metal toolchain missing; downloading (one-time, ~700MB)...');
  runStreaming('xcodebuild', ['-downloadComponent', 'MetalToolchain']);
}

// Fail-closed audit that the freshly built sidecar will load on our macOS floor.
// Cheap static Mach-O checks; any miss aborts before we install a binary that
// would dyld-fail on a user's Mac.
function verifyMacosCompatible(binDir: string): void {
  const server = join(binDir, 'llama-server');
  const metal = join(binDir, 'libggml-metal.0.dylib');
  // arm64 only: the bundled sidecar triple is aarch64-apple-darwin.
  if (run('lipo', ['-archs', server]).trim() !== 'arm64') {
    fail('built llama-server is not an arm64 binary');
  }
  // Deployment target must be exactly the floor, not the build host's newer one.
  const minos = /minos (\d+\.\d+)/.exec(run('otool', ['-l', metal]))?.[1];
  if (minos !== MACOS_DEPLOYMENT_TARGET) {
    fail(`built libggml-metal targets macOS ${minos ?? '?'}, expected ${MACOS_DEPLOYMENT_TARGET}`);
  }
  // The macOS-15 Metal symbol must be a WEAK import, else dyld aborts below 15.
  const residencyLine = run('nm', ['-m', metal])
    .split('\n')
    .find((line) => /residencyset/i.test(line));
  if (residencyLine === undefined || !/weak external/.test(residencyLine)) {
    fail(
      `MTLResidencySetDescriptor is not a weak import; the build would dyld-fail below macOS 15`,
    );
  }
  // No homebrew / non-system dylib deps: absent on user Macs and re-raise the floor.
  if (/\/opt\/homebrew|\/usr\/local/.test(run('otool', ['-L', server]))) {
    fail('built llama-server links non-system dylibs');
  }
}

// Build parallelism capped by available RAM, not just core count. A `-O3` build
// of the heavy ggml/llama translation units needs a few GB each; GitHub's macOS
// runners have only ~7 GB, so an unbounded `-j` (one job per core) OOM-thrashes
// and stalls mid-compile. Reserve ~1 GB for the OS and budget ~3 GB per job, so
// a 7 GB runner uses 2 while a roomy dev machine still uses all its cores.
function buildParallelism(): number {
  const budgetByMemory = Math.floor((totalmem() / 1024 ** 3 - 1) / 3);
  return Math.max(1, Math.min(cpus().length, budgetByMemory));
}

// Clones the pinned commit and builds llama-server. Returns the build's bin/
// directory (llama-server plus its dylib closure), audited macOS-compatible.
function buildFromSource(workDir: string): string {
  const srcDir = join(workDir, 'llama.cpp');
  console.log(
    `ensure-llama-server: building llama.cpp ${LLAMA_CPP_TAG} from source (a few minutes)...`,
  );
  runStreaming('git', [
    'clone',
    '--depth',
    '1',
    '--branch',
    LLAMA_CPP_TAG,
    LLAMA_CPP_REPO,
    srcDir,
  ]);
  const head = run('git', ['-C', srcDir, 'rev-parse', 'HEAD']).trim();
  if (head !== LLAMA_CPP_COMMIT) {
    fail(
      `supply-chain check failed: tag ${LLAMA_CPP_TAG} resolved to ${head}, expected ${LLAMA_CPP_COMMIT}. Refusing to build.`,
    );
  }
  ensureMetalCompiler();
  const buildDir = join(srcDir, 'build');
  runStreaming('cmake', ['-B', buildDir, '-S', srcDir, ...CMAKE_FLAGS]);
  runStreaming('cmake', [
    '--build',
    buildDir,
    '--config',
    'Release',
    '-j',
    String(buildParallelism()),
    '--target',
    'llama-server',
  ]);
  const binDir = join(buildDir, 'bin');
  verifyMacosCompatible(binDir);
  return binDir;
}

if (process.platform !== 'darwin' || process.arch !== 'arm64') {
  console.log(
    `ensure-llama-server: skipping on ${process.platform}/${process.arch} (sidecar is macOS arm64 only)`,
  );
  process.exit(0);
}

// Fast path: pinned version already built. Still re-derive the closure from the
// installed binaries and check the bundle wiring, so an edit to tauri.conf.json
// (or a stale list) fails loudly in dev rather than in the shipped .app.
if (await exists(binPath)) {
  const stamp = await readFile(stampPath, 'utf8').catch(() => '');
  if (stamp.trim() === STAMP_CONTENT) {
    const dylibByName = new Map<string, string>();
    await indexDylibs(destDir, dylibByName);
    await verifyFrameworksList(walkClosure(binPath, dylibByName, DEST));
    process.exit(0);
  }
}

preflightBuildTools();

const workDir = await mkdtemp(join(tmpdir(), 'thuki-llama-'));
try {
  const binDir = buildFromSource(workDir);
  const serverPath = join(binDir, 'llama-server');

  // Index every dylib in the build output, then walk the @rpath link closure
  // from llama-server so we install exactly the dylibs it needs.
  const dylibByName = new Map<string, string>();
  await indexDylibs(binDir, dylibByName);
  const needed = walkClosure(serverPath, dylibByName, 'the build output');

  // Check the bundle wiring before installing anything: a pin bump that changes
  // the closure must update tauri.conf.json in the same change.
  await verifyFrameworksList(needed);

  await mkdir(destDir, { recursive: true });
  await copyFile(serverPath, binPath);
  const installedDylibs: string[] = [];
  for (const name of [...needed].sort()) {
    const target = join(destDir, name);
    // Dereference symlinks: versioned dylib names link to the real file, and the
    // bundle needs regular files.
    await copyFile(await realpath(dylibByName.get(name) as string), target);
    installedDylibs.push(target);
  }

  // At bundle time the sidecar lands in Contents/MacOS while the dylibs go to
  // Contents/Frameworks; the build's @loader_path rpath covers dev (dylibs next
  // to the binary), so add the bundle-relative rpath on top.
  const rpathResult = spawnSync(
    'install_name_tool',
    ['-add_rpath', '@loader_path/../Frameworks', binPath],
    { encoding: 'utf8' },
  );
  if (rpathResult.status !== 0 && !rpathResult.stderr.includes('would duplicate path')) {
    fail(`install_name_tool failed:\n${rpathResult.stderr}`);
  }

  // The rpath edit invalidates the ad-hoc linker signature; re-sign everything
  // we installed so macOS will execute it.
  for (const path of [binPath, ...installedDylibs]) {
    run('codesign', ['--force', '-s', '-', path]);
  }

  await writeFile(stampPath, `${STAMP_CONTENT}\n`, 'utf8');
  console.log(
    `ensure-llama-server: built and installed llama-server ${LLAMA_CPP_TAG} and ${installedDylibs.length} dylibs into ${DEST}`,
  );
} finally {
  await rm(workDir, { recursive: true, force: true });
}
