# Thuki

<p align="center">
  <img src="public/thuki-logo.png" alt="Thuki logo" width="80" />
</p>

<p align="center">
  A floating AI assistant for macOS — fully local, completely free, zero data ever leaves your machine.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License" /></a>
  <a href="https://github.com/quiet-node/thuki/actions/workflows/pr-pipeline.yml"><img src="https://github.com/quiet-node/thuki/actions/workflows/pr-pipeline.yml/badge.svg" alt="CI" /></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey.svg" alt="Platform: macOS" />
</p>

---

**No API keys. No subscriptions. No cloud. No telemetry. Free forever.**

What is Thuki?

Thuki is a lightweight macOS overlay powered by local AI models running entirely on your own machine — built for quick, uninterrupted asks without ever leaving what you're doing.

How to use Thuki?

Highlight a piece of text you have a question about in any app, double-tap Control <kbd>⌃</kbd>, and Thuki floats up right on top — with your selection pre-filled and ready. Ask your question, then save the conversation or toss it away and get straight back to work. No app switching. No breaking your flow. Everything happens in one Space, exactly where you already are.

## Why Thuki?

Most AI assistants require accounts, API keys, or subscriptions that bill you per token. Thuki is different:

- **100% free AI interactions** — you run the model locally, there is no per-query cost, ever
- **Zero trust by design** — no remote server, no cloud backend, no analytics, no telemetry
- **Works completely offline** — once your model is pulled, Thuki runs without an internet connection
- **Your data is yours** — conversations are stored in a local SQLite database on your machine and nowhere else
- **Most importantly: it works everywhere.** Double-tap Control <kbd>⌃</kbd> and Thuki appears — on your desktop, inside a browser, inside a terminal, and yes, even in fullscreen apps. Your favorite AI chat apps can't do that!

## Features

- **Always available** — double-tap Control <kbd>⌃</kbd> to summon the overlay from any app, including fullscreen apps
- **Context-aware quotes** — highlight any text, then double-tap Control <kbd>⌃</kbd> to open Thuki with the selected text pre-filled as a quote
- **Throwaway conversations** — fast, lightweight interactions without the overhead of a full chat app
- **Conversation history** — persist and revisit past conversations across sessions
- **Fully local LLM** — powered by Ollama; no API keys, no accounts, no cost per query
- **Isolated sandbox** — optionally run models in a hardened Docker container with no network egress
- **Image input** — paste or drag screenshots directly into the chat
- **Privacy-first** — zero-trust architecture, all data stays on your device

## Prerequisites: Set Up Your AI Engine First

Before installing Thuki, you need a local AI model running. Choose one of the two options below.

### Option A: Local Ollama (Recommended for most users)

[Ollama](https://ollama.com) runs AI models directly on your Mac. It's free, open-source, and takes about 5 minutes to set up.

1. **Install Ollama**

   Download and install from [ollama.com](https://ollama.com), or via Homebrew:

   ```bash
   brew install ollama
   ```

2. **Pull a model**

   ```bash
   ollama pull gemma3:4b
   ```

   > **Note:** Model files are large (typically 2–8 GB). This step can take several minutes depending on your internet connection. You only need to do it once.

3. **Verify the model is ready**

   ```bash
   ollama list
   ```

   You should see your model listed. Once it appears, Ollama is ready and Thuki will connect to it automatically at `http://127.0.0.1:11434`.

### Option B: Docker Sandbox (For security-conscious users)

The Docker sandbox is for users who want the strongest possible isolation between the AI model and their host system — ideal if you work in regulated environments, are security-conscious about what runs on your machine, or simply want peace of mind.

The sandbox runs Ollama inside a hardened container with:

- **Network air-gap** — the container runs on an internal bridge network with zero internet egress. The model cannot make outbound connections of any kind.
- **Privilege dropping** — all Linux kernel capabilities are dropped (`cap_drop: ALL`). The container runs with the minimum possible privileges.
- **Read-only model weights** — model files are mounted read-only (`:ro`). A malicious prompt cannot modify or persist changes to the model.
- **Ephemeral state** — all model data is wiped on shutdown (`docker compose down -v`). Nothing persists between sessions.

**Prerequisites:** Install [Docker Desktop](https://www.docker.com/get-started) for Mac.

The sandbox is started as part of the Thuki launch process — see [Installation](#installation) below.

---

## Installation

### Download (Recommended)

1. Download `Thuki.app.tar.gz` from the [latest release](https://github.com/quiet-node/thuki/releases/latest)
2. Extract and move `Thuki.app` to your `/Applications` folder
3. Remove the macOS quarantine flag (required for unsigned apps):

   ```bash
   xattr -rd com.apple.quarantine /Applications/Thuki.app
   ```

4. Open Thuki — it will appear in your menu bar

> **First launch:** macOS will ask for Accessibility permission. This is required for the global keyboard shortcut that lets you summon Thuki from any app. Grant it once — it persists across restarts.

If you chose the **Docker sandbox**, start it now:

```bash
bun run sandbox:start
```

### Build from Source

**Prerequisites:** [Bun](https://bun.sh), [Rust](https://rustup.rs), and optionally [Docker](https://www.docker.com/get-started)

```bash
# Clone and install dependencies
git clone https://github.com/quiet-node/thuki.git
cd thuki
bun install

# If using the Docker sandbox, start it now
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

## Author

Built by [@quiet_node](https://x.com/quiet_node).

## License

Copyright 2026 Quiet Node Contributors. Licensed under the [Apache License, Version 2.0](LICENSE).
