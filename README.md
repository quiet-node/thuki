# Thuki

<p align="center">
  <img src="public/thuki-logo.png" alt="Thuki logo" width="300" />
</p>

<p align="center">
  A floating AI secretary for macOS. Fully local, completely free, zero data ever leaves your machine.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-beta-yellow.svg" alt="Beta" />
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License" /></a>
  <a href="https://github.com/quiet-node/thuki/actions/workflows/pr-pipeline.yml"><img src="https://github.com/quiet-node/thuki/actions/workflows/pr-pipeline.yml/badge.svg" alt="CI" /></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey.svg" alt="Platform: macOS" />
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Tauri-v2-24C8DB?logo=tauri&logoColor=white" alt="Tauri v2" />
  <img src="https://img.shields.io/badge/React-19-61DAFB?logo=react&logoColor=black" alt="React 19" />
  <img src="https://img.shields.io/badge/TypeScript-5.8-3178C6?logo=typescript&logoColor=white" alt="TypeScript" />
  <img src="https://img.shields.io/badge/Rust-stable-CE422B?logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/Tailwind_CSS-4-06B6D4?logo=tailwindcss&logoColor=white" alt="Tailwind CSS 4" />
  <img src="https://img.shields.io/badge/SQLite-bundled-003B57?logo=sqlite&logoColor=white" alt="SQLite" />
  <img src="https://img.shields.io/badge/Ollama-local-black" alt="Ollama" />
</p>

---

⚠️ **BETA: Active Development.** This project is in active development. Features may change, bugs may occur, and AI model outputs are not guaranteed to be perfect or accurate. Use at your own risk. Always verify important information with trusted sources.

---

**No API keys. No subscriptions. No cloud. No telemetry. Free forever.**

What is Thuki?

Thuki is a lightweight macOS overlay powered by local AI models running entirely on your own machine, built for quick, uninterrupted asks without ever leaving what you're doing.

How to use Thuki?

Highlight a piece of text you have a question about in any app, double-tap Control <kbd>⌃</kbd>, and Thuki floats up right on top, with your selection pre-filled and ready. Ask your question, then save the conversation or toss it away and get straight back to work. No app switching. No breaking your flow. Everything happens in one Space, exactly where you already are.

## Why Thuki?

Most AI tools require accounts, API keys, or subscriptions that bill you per token. Thuki is different:

- **100% free AI interactions:** you run the model locally, there is no per-query cost, ever
- **Zero trust by design:** no remote server, no cloud backend, no analytics, no telemetry
- **Works completely offline:** once your model is pulled, Thuki runs without an internet connection
- **Your data is yours:** conversations are stored in a local SQLite database on your machine and nowhere else
- **Most importantly: it works everywhere.** Double-tap Control <kbd>⌃</kbd> and Thuki appears on your desktop, inside a browser, inside a terminal, and yes, even in fullscreen apps. Your favorite AI chat apps can't do that!

## Features

- **Always available:** double-tap Control <kbd>⌃</kbd> to summon the overlay from any app, including fullscreen apps
- **Context-aware quotes:** highlight any text, then double-tap Control <kbd>⌃</kbd> to open Thuki with the selected text pre-filled as a quote
- **Throwaway conversations:** fast, lightweight interactions without the overhead of a full chat app
- **Conversation history:** persist and revisit past conversations across sessions
- **Fully local LLM:** powered by Ollama; no API keys, no accounts, no cost per query
- **Isolated sandbox:** optionally run models in a hardened Docker container with capability dropping, read-only volumes, and localhost-only networking
- **Image input:** paste or drag screenshots directly into the chat
- **Privacy-first:** zero-trust architecture, all data stays on your device

## Getting Started

### Step 1: Set Up Your AI Engine

> **Default model:** Thuki ships with [`gemma3:4b`](https://ollama.com/library/gemma3) by default, a capable 4-billion parameter model from Google that runs comfortably on most modern Macs with 8 GB of RAM or more. It's a great starting point: fast, conversational, and surprisingly capable for everyday tasks.

Support for swapping models without rebuilding is on the roadmap; see [What's Next](#whats-next-for-thuki).

Choose one of the two options below to set up your AI engine before installing Thuki.

#### Option A: Local Ollama (Recommended for most users)

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

#### Option B: Docker Sandbox (For security-conscious users)

**Prerequisites:** Install [Docker Desktop](https://www.docker.com/get-started)

The Docker sandbox is for users who want the strongest possible isolation between the AI model and their host system, ideal if you work in regulated environments, are security-conscious about what runs on your machine, or simply want peace of mind. The model runs in a hardened container that cannot reach the internet, cannot write to your filesystem, and leaves no trace when stopped.

Start the sandbox:

```bash
bun run sandbox:start
```

> **First run:** The sandbox will pull the model inside the container; this may take several minutes depending on your connection. Subsequent starts are instant.

When you're done, stop and wipe all model data:

```bash
bun run sandbox:stop
```

For the full architecture and security philosophy behind the sandbox, see [`sandbox/README.md`](sandbox/README.md).

### Step 2: Install Thuki

#### Download (Recommended)

1. Download `Thuki.app.tar.gz` from the [latest release](https://github.com/quiet-node/thuki/releases/latest)
2. Extract the archive and move `Thuki.app` to your `/Applications` folder
3. **Before opening Thuki for the first time**, run this command in Terminal:

   ```bash
   xattr -rd com.apple.quarantine /Applications/Thuki.app
   ```

   > **Why is this needed?** Thuki is an open-source app distributed directly — not through the Mac App Store. Apple's Gatekeeper automatically blocks any app downloaded from the internet that hasn't gone through Apple's paid notarization process ($99/year Apple Developer account). This one-time command removes that block. It is safe and [officially documented by Apple](https://support.apple.com/en-us/102445).
   >
   > **What happens if you skip this step and open Thuki first?** macOS will show a dialog saying "Apple could not verify Thuki is free of malware." Click **Done** (not "Move to Trash"), then go to **System Settings → Privacy & Security**, scroll down until you see "Thuki was blocked", and click **Open Anyway**. Enter your Mac password when prompted, then open Thuki again.

4. Open Thuki. It will appear in your menu bar.

> **First launch:** macOS will ask for Accessibility permission. This is required for the global keyboard shortcut that lets you summon Thuki from any app. Grant it once; it persists across restarts.

#### Build from Source

**Prerequisites:** [Bun](https://bun.sh), [Rust](https://rustup.rs), and optionally [Docker](https://www.docker.com/get-started)

```bash
# Clone and install dependencies
git clone https://github.com/quiet-node/thuki.git
cd thuki
bun install

# Launch in development mode
bun run dev
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development setup guide.

## Architecture & Security

<details>
<summary>Click to expand</summary>

Thuki is a **Tauri v2** app (Rust backend + React/TypeScript frontend) that interfaces with a locally running Ollama instance at `http://127.0.0.1:11434`.

### Dual-Layer Isolation

1. **Frontend (Tauri/React):** Operates within a secure system webview with restricted IPC. Streaming uses Tauri's Channel API; the Rust backend sends typed `StreamChunk` enum variants, and the frontend hook accumulates tokens into React state.

2. **Generative Engine (Docker Sandbox):**
   - **Ingress Isolation:** The API is bound to `127.0.0.1` only, blocking all external network access
   - **Privilege Dropping:** All Linux kernel capabilities are dropped (`cap_drop: ALL`)
   - **Model Integrity:** Model weights are mounted read-only (`:ro`) to prevent tampering
   - **Ephemeral State:** All model data is purged on shutdown via `docker compose down -v`

### Window Lifecycle

The app starts hidden. The hotkey or tray menu shows it. The window close button hides (not quits); quit is only available from the tray. `ActivationPolicy::Accessory` hides the Dock icon. `macOSPrivateApi: true` enables NSPanel for fullscreen-app overlay.

</details>

## Configuration

See [docs/configurations.md](docs/configurations.md) for the full configuration reference (quote display limits and system prompt).

## Contributing

Contributions are welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) to get started. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Author

Reach out to [Logan](https://x.com/quiet_node) on X with questions or feedback.

## What's next for Thuki

Thuki is just getting started. Here's where it's headed:

### Secretary Superpowers

The big leap: from answering questions to taking action.

- **Internet search:** let Thuki look things up in real time, not just reason from its training data
- **Tool integrations via [MCP](https://modelcontextprotocol.io/):** connect Thuki to Gmail, Slack, Discord, Google Calendar, and any other MCP-compatible service; ask it to draft a reply, summarize a thread, or schedule a meeting without ever leaving your current app
- **Slash commands:** type `/summarize`, `/translate`, `/explain`, `/rewrite`, and more to instantly trigger built-in prompts without typing a full question

### Better AI Control

More flexibility over the model powering Thuki.

- **Native settings panel (⌘,):** a proper macOS preferences window to configure your model, Ollama endpoint, activation shortcut, slash commands, and system prompt. No config files needed.
- **In-app model switching:** swap between any Ollama model from the UI without rebuilding
- **Multiple provider support:** opt in to OpenAI, Anthropic, or any OpenAI-compatible endpoint as an alternative to local Ollama
- **Custom activation shortcut:** change the double-tap trigger to any key or combo you prefer

### Richer Context

Give Thuki more to work with.

- **Voice input:** dictate your question instead of typing
- **Auto-capture screen context:** activate Thuki and have it automatically read the active window or selected region as context
- **File and document drop:** drag a PDF, image, or text file directly into Thuki as context for your question

---

Have a feature idea? [Open an issue](https://github.com/quiet-node/thuki/issues) and let's talk about it.

## License

Copyright 2026 Logan Nguyen. Licensed under the [Apache License, Version 2.0](LICENSE).
