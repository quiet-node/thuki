# Contributing to Thuki

Thank you for your interest in contributing to [Thuki](https://www.thuki.app/)! This guide will walk you through everything you need to get started.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Development Setup](#development-setup)
- [Running Tests](#running-tests)
- [Code Style](#code-style)
- [Submitting a Pull Request](#submitting-a-pull-request)
- [Good First Issues](#good-first-issues)

---

## Prerequisites

You'll need the following tools installed before you can build Thuki:

### Required

**Bun:** JavaScript runtime and package manager

```bash
curl -fsSL https://bun.sh/install | bash
```

**Rust:** required for the Tauri backend

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

After installation, restart your shell or run `source ~/.cargo/env` to make `cargo` available. Thuki builds against stable Rust.

Running the coverage suite (required before submitting a PR) also needs the `nightly-2026-03-30` toolchain with `llvm-tools`:

```bash
rustup toolchain install nightly-2026-03-30 --component llvm-tools
```

**macOS:** Thuki is macOS-only. It uses NSPanel and Core Graphics APIs that are not available on other platforms.

**CMake:** required to build the bundled llama.cpp inference engine from source

```bash
brew install cmake
```

**Xcode:** full Xcode.app (from the App Store) is required, not just the Command Line Tools. The engine build needs `xcodebuild` to fetch the Metal toolchain that compiles its GPU shaders, and `xcodebuild` refuses to run when the active developer directory is the Command Line Tools. After installing Xcode, point `xcode-select` at it:

```bash
sudo xcode-select -s /Applications/Xcode.app
```

### Optional

No AI backend setup is required: Thuki bundles its own llama.cpp inference engine, and the dev/build scripts build the pinned `llama-server` sidecar from source automatically (this uses the CMake and Xcode prerequisites above; see Development Setup below). Install these only if you want to develop against an alternative provider:

**Ollama:** to test the Ollama provider against a native install

- Install via [ollama.com](https://ollama.com)

---

## Development Setup

1. **Fork and clone the repository**

   ```bash
   git clone https://github.com/quiet-node/thuki.git
   cd thuki
   ```

2. **Install frontend dependencies**

   ```bash
   bun install
   ```

3. **AI engine: built automatically**

   Thuki bundles its own inference engine (llama.cpp's `llama-server`). On a fresh clone, the first `bun run dev` (or `build:backend` / `build:release`) automatically runs `bun run engine:ensure`, which clones the pinned llama.cpp tag, verifies the commit, then builds and installs the binary and its dylibs under `src-tauri/binaries/` (gitignored). That first build compiles llama.cpp from source, so it takes a while and needs the CMake and Xcode prerequisites above; later runs are an instant no-op until the pin changes. You pick and download a starter model inside the app's onboarding flow.

   **Optional: develop against an alternative provider**

   To test the Ollama provider, run a native Ollama install with a model pulled (`ollama pull gemma4:e2b`; Thuki's Ollama provider defaults to `http://127.0.0.1:11434`).

4. **Configuration** (optional)

   Thuki writes a default config file to `~/Library/Application Support/com.quietnode.thuki/config.toml` on first launch. To customize anything (model, system prompt, window dimensions, activation timing, quote display), edit that file and relaunch. See [docs/configurations.md](docs/configurations.md) for the full schema.

   Built-in web search is keyless and in-process: the pipeline lives in `src-tauri/src/websearch/` with SSRF-safe HTTP in `src-tauri/src/net/`. Design handbook: [docs/built-in-web-search.md](docs/built-in-web-search.md). Privacy egress: [docs/search-privacy.md](docs/search-privacy.md). Constants: [docs/configurations.md](docs/configurations.md). Manual live eval harnesses (ignored by default CI): [docs/search-eval.md](docs/search-eval.md).

5. **Launch the app**

   ```bash
   bun run dev
   ```

   On first run, macOS will prompt for Accessibility permission. This is required for the global keyboard shortcut. Grant it once; it persists across restarts.

   **Dev runs and permissions:** `bun run dev` launches a bare binary (`src-tauri/target/debug/thuki`), not an app bundle, so macOS attributes Accessibility and Screen Recording grants to the terminal app running the command (Terminal.app, iTerm, VS Code, etc.), not to Thuki. Grant both permissions to that terminal app. Switching terminal apps means re-granting.

---

## Running Tests

**100% code coverage is mandatory.** All new or modified code must maintain 100% coverage across lines, functions, branches, and statements. PRs that drop below 100% will not be merged.

### Frontend tests (Vitest + React Testing Library)

```bash
bun run test              # Run all frontend tests
bun run test:watch        # Watch mode
bun run test:coverage     # Run with coverage report
```

Coverage output is in `coverage/`. Open `coverage/index.html` in a browser for a visual breakdown.

### Backend tests (Cargo)

```bash
bun run test:backend           # Run all Rust tests
bun run test:backend:coverage  # Run with 100% line coverage enforcement
```

### Run everything

```bash
bun run test:all           # Both frontend and backend tests
bun run test:all:coverage  # Both with coverage enforcement
```

### Full validation gate

Before submitting a PR, run the full validation suite:

```bash
bun run validate-build
```

This runs lint, format check, typecheck, and build in sequence. All must pass with zero warnings and zero errors.

---

## Code Style

**Formatting and linting are enforced by CI.** To avoid failed PR checks, run these locally before pushing:

```bash
bun run format   # Auto-format TypeScript/CSS (Prettier) and Rust (cargo fmt)
bun run lint     # ESLint + cargo clippy
```

Key style rules:
- TypeScript: enforced by ESLint with `@eslint-react` rules
- Rust: enforced by `cargo clippy -- -D warnings` (warnings are errors)
- No `console.log` or debug output in committed code

---

## Submitting a Pull Request

1. **Create a branch** from `main`

   ```bash
   git checkout -b feat/your-feature-name
   ```

2. **Make your changes** following the code style guidelines above

3. **Write or update tests** to maintain 100% coverage

4. **Run the validation suite**

   ```bash
   bun run test:all:coverage
   bun run validate-build
   ```

5. **Commit your changes** using [Conventional Commits](https://www.conventionalcommits.org/) format:

   ```
   <type>: <short description>
   ```

   Common types: `feat` (new feature), `fix` (bug fix), `docs` (documentation), `refactor`, `test`, `chore`. Keep the subject line under 72 characters.

6. **Open a PR** against `main` and fill out the PR template fully

7. **Respond to review feedback:** maintainers aim to review within a few days

### PR Guidelines

- Keep PRs focused on a single change. Large, multi-concern PRs are harder to review and slower to merge.
- If you're fixing a bug, include a test that would have caught the bug.
- If you're adding a feature, document it in `docs/configurations.md` if it's configurable.
- Link any related issues in the PR description.

---

## Good First Issues

New to the codebase? Look for issues tagged [`good first issue`](https://github.com/quiet-node/thuki/issues?q=is%3Aopen+is%3Aissue+label%3A%22good+first+issue%22) on GitHub. These are scoped to be approachable without deep knowledge of the full system.

If you have a question or want to discuss an approach before writing code, open an issue or start a discussion; we're happy to help.
