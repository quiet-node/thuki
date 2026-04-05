# Thuki

<p align="center">
  <img src="public/thuki-logo.png" alt="Thuki logo" width="80" />
</p>

<p align="center">
  The context-aware floating secretary — a private, local AI overlay for macOS.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License" /></a>
  <a href="https://github.com/quiet-node/thuki/actions/workflows/pr-pipeline.yml"><img src="https://github.com/quiet-node/thuki/actions/workflows/pr-pipeline.yml/badge.svg" alt="CI" /></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey.svg" alt="Platform: macOS" />
</p>

---

Thuki is a lightweight macOS desktop app that floats above your workspace. Double-tap Control to summon it from any app, ask a question, and get back to work. It connects to a locally running [Ollama](https://ollama.com) instance — your data never leaves your machine.

## Features

- **Always available** — double-tap Control to summon the overlay from any app, including fullscreen apps
- **Fully local** — powered by Ollama; no cloud, no telemetry, no API keys required
- **Isolated sandbox** — optionally run models in a hardened Docker container with no network egress
- **Conversation history** — persist and revisit past conversations across sessions
- **Image input** — paste or drag screenshots directly into the chat
- **Privacy-first** — zero-trust architecture, all data stays on your device

## Installation

### Download (Recommended)

1. Download `Thuki.app.tar.gz` from the [latest release](https://github.com/quiet-node/thuki/releases/latest)
2. Extract and move `Thuki.app` to your `/Applications` folder
3. Remove the macOS quarantine flag (required for unsigned apps):

   ```bash
   xattr -rd com.apple.quarantine /Applications/Thuki.app
   ```

4. Make sure [Ollama](https://ollama.com) is running locally with a model pulled, then open Thuki

> **First launch:** macOS will ask for Accessibility permission. This is required for the global keyboard shortcut. Grant it once — it persists across restarts.

### Build from Source

**Prerequisites:** [Bun](https://bun.sh), [Rust](https://rustup.rs), and optionally [Docker](https://www.docker.com/get-started)

```bash
# Clone and install dependencies
git clone https://github.com/quiet-node/thuki.git
cd thuki
bun install

# Start the Docker sandbox (optional — skip if you have Ollama running locally)
bun run sandbox:start

# Launch in development mode
bun run dev
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development setup guide.

## Architecture & Security

<details>
<summary>Click to expand</summary>

Thuki is a **Tauri v2** app (Rust backend + React/TypeScript frontend) that interfaces with a locally running Ollama instance at `http://127.0.0.1:11434`.

### Dual-Layer Isolation

1. **Frontend (Tauri/React):** Operates within a secure system webview with restricted IPC. Streaming uses Tauri's Channel API — the Rust backend sends typed `StreamChunk` enum variants, and the frontend hook accumulates tokens into React state.

2. **Generative Engine (Docker Sandbox):**
   - **Network Air-Gap:** Runs in an internal bridge network with zero internet egress (`internal: true`)
   - **Privilege Dropping:** All Linux kernel capabilities are dropped (`cap_drop: ALL`)
   - **Model Integrity:** Model weights are mounted read-only (`:ro`) to prevent tampering
   - **Ephemeral State:** All model data is purged on shutdown via `docker compose down -v`

### Window Lifecycle

The app starts hidden. The hotkey or tray menu shows it. The window close button hides (not quits); quit is only available from the tray. `ActivationPolicy::Accessory` hides the Dock icon. `macOSPrivateApi: true` enables NSPanel for fullscreen-app overlay.

</details>

## Configuration

See [docs/configurations.md](docs/configurations.md) for the full configuration reference, including how to change the default model and Ollama URL.

## Contributing

Contributions are welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) to get started. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

Copyright 2024 Quiet Node Contributors. Licensed under the [Apache License, Version 2.0](LICENSE).
