# Contributing to Thuki

Thank you for your interest in contributing! This guide will walk you through everything you need to get started.

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

**macOS:** Thuki is macOS-only. It uses NSPanel and Core Graphics APIs that are not available on other platforms.

### Optional

**Docker:** only needed if you want to run the isolated Docker sandbox instead of a local Ollama install

- Install via [docker.com](https://www.docker.com/get-started)

**Ollama:** if you're not using the Docker sandbox, install Ollama directly

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

3. **Set up your AI backend** (choose one):

   **Option A: Docker sandbox (recommended for isolation)**

   ```bash
   bun run sandbox:start
   ```

   This pulls the default model (`gemma3:4b`) and starts an air-gapped container. It may take a few minutes on first run.

   **Option B: Local Ollama**

   Make sure Ollama is running and you have a model pulled:

   ```bash
   ollama pull gemma3:4b
   ```

   Thuki connects to `http://127.0.0.1:11434` by default.

4. **Configure environment** (optional)

   ```bash
   cp .env.example .env
   ```

   Edit `.env` to customize quote display behavior or the system prompt. See [docs/configurations.md](docs/configurations.md) for all available options.

5. **Launch the app**

   ```bash
   bun run dev
   ```

   On first run, macOS will prompt for Accessibility permission. This is required for the global keyboard shortcut. Grant it once; it persists across restarts.

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

5. **Commit your changes** with a clear, descriptive message

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
