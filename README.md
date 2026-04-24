<h1 align="center">
  Thuki - WIP
</h1>

<p align="center">
  <img src="public/thuki-logo.png" alt="Thuki logo" width="300" />
</p>

<p align="center">
  <a href="https://www.producthunt.com/products/thuki?embed=true&amp;utm_source=badge-featured&amp;utm_medium=badge&amp;utm_campaign=badge-thuki" target="_blank" rel="noopener noreferrer"><img alt="Thuki  - Floating AI for macOS. Free &amp; Local. No cloud, no API keys. | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1122707&amp;theme=light&amp;t=1776150241085">  </a>  
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

**No API keys. No subscriptions. No cloud. No telemetry. Free forever.**

Thuki (thư kí - Vietnamese for secretary) is a lightweight macOS overlay powered by local AI models running entirely on your own machine, built for quick, uninterrupted asks without ever leaving what you're doing.

## See It in Action

### Basic Usage

Double-tap Control <kbd>⌃</kbd> to summon Thuki from anywhere. Ask a question, get an answer, and dismiss. Use `/screen` or the screenshot button to capture your screen and attach it as context.

https://github.com/user-attachments/assets/57df0efe-24eb-4875-a83d-e605e0c6f8b4

### Overlay Mode

Thuki floats above every app, including fullscreen ones. Highlight text anywhere, double-tap Control <kbd>⌃</kbd>, and Thuki opens with your selection pre-filled as a quote, ready to ask about.

https://github.com/user-attachments/assets/f52b55f7-479d-4c2e-a361-1553fe132712

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
- **Image input:** paste or drag images and screenshots directly into the chat
- **Screen capture:** type `/screen` to instantly capture your entire screen and attach it to your question as context
- **Slash commands:** built-in commands for live search and prompt shortcuts: `/search`, `/translate`, `/rewrite`, `/tldr`, `/refine`, `/bullets`, `/todos`. Highlight text anywhere, summon Thuki, type a command, and hit Enter
- **Extended reasoning:** type `/think` to have the model reason through a problem step by step before answering
- **Privacy-first:** zero-trust architecture, all data stays on your device

## Getting Started

### Step 1: Set Up Your AI Engine

> **Default model:** Thuki ships with [`gemma4:e2b`](https://ollama.com/library/gemma4) by default, an effective 2B parameter edge model from Google. It runs comfortably on most modern Macs with 8 GB of RAM and delivers strong performance on reasoning, coding, and vision tasks. To use a different model, edit `~/Library/Application Support/com.quietnode.thuki/config.toml` and reorder the `[model] available` list so your preferred model is first. See [Configurations](docs/configurations.md) for the full schema.

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
   ollama pull gemma4:e2b
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

### Step 2: Setup the search sandbox (Optional, required for /search)

The `/search` command uses an agentic search pipeline that depends on two local Docker containers: a **SearXNG** meta-search engine and a **Trafilatura** reader. This setup ensures that your search queries and the content you read remain entirely local.

**Prerequisite:** [Docker Desktop](https://www.docker.com/get-started) must be running.

1. **Start the search services**

   ```bash
   bun run search-box:start
   ```

2. **Verify services (Optional)**

   ```bash
   # Search Engine check:
   curl "http://127.0.0.1:25017/search?q=thuki&format=json"
   ```

   Without this service running, the `/search` command will be disabled in the chat, but all other features will remain available.

   For more details on the agentic search pipeline, see [docs/agentic-search.md](docs/agentic-search.md).

### Step 3: Install Thuki

#### Download (Recommended)

1. Download `Thuki.dmg` from the [latest release](https://github.com/quiet-node/thuki/releases/latest)
2. Double-click `Thuki.dmg` to open it. A window appears showing the Thuki app icon next to an Applications folder shortcut.
3. Drag `Thuki` onto the `Applications` folder shortcut.
4. Eject the disk image (drag it to Trash in the Finder sidebar, or right-click and choose Eject).
5. **Before opening Thuki for the first time**, run this command in Terminal:

   ```bash
   xattr -rd com.apple.quarantine /Applications/Thuki.app
   ```

   > **Why is this needed?** Thuki is a free, non-profit, open-source app distributed directly and not through the Mac App Store. Apple's Gatekeeper automatically blocks any app downloaded from the internet that has not gone through Apple's paid notarization process. This one-time command removes that block. It is safe and [officially documented by Apple](https://support.apple.com/en-us/102445).

6. Open Thuki. It will appear in your menu bar.

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

See [docs/commands.md](docs/commands.md) for the full slash command reference.

## Contributing

Contributions are welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) to get started. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Community Ports

Thuki is macOS-only, but the community has been busy bringing it to other platforms. Huge shoutout to these contributors 🎊🚀!

| Platform      | Repo                                               | Author                                       |
| ------------- | -------------------------------------------------- | -------------------------------------------- |
| Windows 10/11 | [ThukiWin](https://github.com/ayzekhdawy/thukiwin) | [@ayzekhdawy](https://github.com/ayzekhdawy) |

> Each port is independently maintained by its author. For issues or questions about a specific port, head to that repo directly.

## Author

Reach out to [Logan](https://x.com/quiet_node) on X with questions or feedback.

## What's next for Thuki

Thuki is just getting started. Here's where it's headed:

### Secretary Superpowers

The big leap: from answering questions to taking action.

- **Internet search:** let Thuki look things up in real time, not just reason from its training data
- **Tool integrations via [MCP](https://modelcontextprotocol.io/):** connect Thuki to Gmail, Slack, Discord, Google Calendar, and any other MCP-compatible service; ask it to draft a reply, summarize a thread, or schedule a meeting without ever leaving your current app
- **More slash commands:** `/screen`, `/think`, `/translate`, `/rewrite`, `/tldr`, `/refine`, `/bullets`, and `/todos` are live. More domain-specific commands are on the way

### Better AI Control

More flexibility over the model powering Thuki.

- **Native settings panel (⌘,):** a proper macOS preferences window to configure your model, Ollama endpoint, activation shortcut, slash commands, and system prompt. No config files needed.
- **In-app model switching:** swap between any Ollama model from the UI without restarting (the backend already supports multiple models via the `[model] available` list in `config.toml`; the picker UI is next)
- **Multiple provider support:** opt in to OpenAI, Anthropic, or any OpenAI-compatible endpoint as an alternative to local Ollama
- **Custom activation shortcut:** change the double-tap trigger to any key or combo you prefer

### Richer Context

Give Thuki more to work with.

- **Voice input:** dictate your question instead of typing
- **Auto-capture screen context:** activate Thuki and have it automatically read the active window or selected region as context (partial: `/screen` captures the full screen today; targeted region capture is next)
- **File and document drop:** drag a PDF, image, or text file directly into Thuki as context for your question

---

Have a feature idea? [Open an issue](https://github.com/quiet-node/thuki/issues) and let's talk about it.

## License

Copyright 2026 Logan Nguyen. Licensed under the [Apache License, Version 2.0](LICENSE).
