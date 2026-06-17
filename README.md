<h1 align="center">
  Thuki - WIP
</h1>

<p align="center">
  <a href="https://www.thuki.app/" target="_blank" rel="noopener noreferrer"><img src="public/thuki-logo.png" alt="Thuki logo" width="300" /></a>
</p>

<!-- <p align="center">
  <a href="https://www.producthunt.com/products/thuki?embed=true&amp;utm_source=badge-featured&amp;utm_medium=badge&amp;utm_campaign=badge-thuki" target="_blank" rel="noopener noreferrer"><img alt="Thuki  - Floating AI for macOS. Free &amp; Local. No cloud, no API keys. | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1122707&amp;theme=light&amp;t=1776150241085">  </a>  
</p> -->
 <p align="center">
<a href="https://www.star-history.com/?repos=quiet-node%2Fthuki">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=quiet-node/thuki&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=quiet-node/thuki&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=quiet-node/thuki&type=date&legend=top-left" />
 </picture>
</a>
 </p>

<p align="center">
  A floating AI secretary for macOS. Fully local, completely free, zero data ever leaves your machine.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-beta-yellow.svg" alt="Beta" />
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License" /></a>
  <a href="https://www.thuki.app/" target="_blank" rel="noopener noreferrer"><img src="https://img.shields.io/badge/thuki.app-000000?style=flat" alt="thuki.app" /></a>
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
- **Image input:** paste or drag images and screenshots directly into the chat
- **Screen capture:** type `/screen` to instantly capture your entire screen and attach it to your question as context
- **OCR on text-only models:** `/extract`, `/explain`, `/tldr`, `/translate`, `/rewrite`, `/refine`, `/bullets`, and `/todos` read attached images via macOS Vision OCR, so they work even when the active model has no vision capability
- **Agentic search:** type `/search` to run a fully local, multi-step search pipeline (SearXNG + Trafilatura reader) with a live trace of every query, fetch, and judgement step
- **Slash commands:** built-in commands for live search and prompt shortcuts: `/search`, `/extract`, `/explain`, `/translate`, `/rewrite`, `/tldr`, `/refine`, `/bullets`, `/todos`. Highlight text anywhere, summon Thuki, type a command, and hit Enter
- **Extended reasoning:** type `/think` to have the model reason through a problem step by step before answering
- **Math rendering:** LaTeX expressions in responses render as formatted equations via KaTeX
- **In-app model picker:** browse the models installed in your local Ollama and switch the active model from the ask bar without ever opening a config file
- **Cross-model continuity:** swap models mid-conversation and Thuki sanitizes history and filters capabilities (vision, thinking) to whatever the new model supports
- **Settings panel:** a four-tab native window (⌘,) for inference, prompt, window, and search settings, including a log-scale context-window slider and a tunable image-attachment cap (up to 20)
- **Contextual tip bar:** lightweight in-overlay hints surface the right shortcut or command at the right moment
- **Privacy-first:** zero-trust architecture, all data stays on your device

## Getting Started

### Step 1: Set Up Your AI Engine

Set up [Ollama](https://ollama.com) to run AI models directly on your Mac before installing Thuki. It's free, open-source, and takes about 5 minutes to set up.

1. **Install Ollama**

   Download and install from [ollama.com](https://ollama.com), or via Homebrew:

   ```bash
   brew install ollama
   ```

2. **Pull a model**

   ```bash
   ollama pull gemma4:e2b
   ```

   > **Note:** Model files are large (typically 2–8 GB). This step can take several minutes depending on your internet connection. You only need to do it once. Any model in the [Ollama library](https://ollama.com/library) works; `gemma4:e2b` is the recommended starting point. Pull additional models anytime and switch between them from Thuki's ask bar.

3. **Verify the model is ready**

   ```bash
   ollama list
   ```

   You should see your model listed. Once it appears, Ollama is ready and Thuki will connect to it automatically at `http://127.0.0.1:11434`.

### Step 2: Install Thuki

#### Download (Recommended)

1. Download `Thuki.dmg` from the [latest stable release](https://github.com/quiet-node/thuki/releases/latest), or grab the bleeding-edge build from the [`nightly`](https://github.com/quiet-node/thuki/releases/tag/nightly) channel which is rebuilt automatically from `main`.
2. Double-click `Thuki.dmg` to open it. A window appears showing the Thuki app icon next to an Applications folder shortcut.
3. Drag `Thuki` onto the `Applications` folder shortcut.
4. Eject the disk image (drag it to Trash in the Finder sidebar, or right-click and choose Eject).
5. **Before opening Thuki for the first time**, run this command in Terminal:

   ```bash
   xattr -rd com.apple.quarantine /Applications/Thuki.app
   ```

   > **Why is this needed?** Thuki is a free, non-profit, open-source app distributed directly and not through the Mac App Store. Apple's Gatekeeper automatically blocks any app downloaded from the internet that has not gone through Apple's paid notarization process. This one-time command removes that block. It is safe and [officially documented by Apple](https://support.apple.com/en-us/102445).

6. Open Thuki. It will appear in your menu bar.

> **First launch:** macOS will ask for two permissions. **Accessibility** is required for the global keyboard shortcut that lets you summon Thuki from any app. **Screen Recording** is required for the `/screen` command and the screenshot button. Grant both once; they persist across restarts.

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

### Optional: Enable `/search`

The `/search` command runs a fully local agentic search pipeline backed by two Docker services (SearXNG + a Trafilatura reader). It is **not bundled with the `.dmg`**: enabling it currently requires cloning this repository to run the local services. Every other Thuki feature works without it. First-class, out-of-box `/search` support is on the roadmap.

See [docs/agentic-search.md#setup](docs/agentic-search.md#setup) for the setup steps.

## Architecture & Security

<details>
<summary>Click to expand</summary>

Thuki is a **Tauri v2** app (Rust backend + React/TypeScript frontend) that interfaces with a locally running Ollama instance at `http://127.0.0.1:11434`.

### Frontend Isolation

The frontend operates within a secure system webview with restricted IPC. Streaming uses Tauri's Channel API; the Rust backend sends typed `StreamChunk` enum variants, and the frontend hook accumulates tokens into React state.

### Window Lifecycle

The app starts hidden. The hotkey or tray menu shows it. The window close button hides (not quits); quit is only available from the tray. `ActivationPolicy::Accessory` hides the Dock icon. `macOSPrivateApi: true` enables NSPanel for fullscreen-app overlay.

</details>

## Configuration

Thuki reads a single typed TOML file at `~/Library/Application Support/com.quietnode.thuki/config.toml`, seeded with sensible defaults on first launch. The in-app Settings panel (⌘,) writes to the same file, so you can edit by hand or click through tabs, whichever you prefer.

See [docs/configurations.md](docs/configurations.md) for the full schema covering the `[inference]`, `[prompt]`, `[window]`, `[quote]`, and `[search]` sections (Ollama URL, system prompt, context window, image cap, agentic-search timeouts, and more).

See [docs/commands.md](docs/commands.md) for the full slash command reference, and [docs/tuning-context-window.md](docs/tuning-context-window.md) for guidance on picking a `num_ctx` value.

## Contributing

Contributions are welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) to get started. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Community Ports

Thuki is macOS-only, but the community has been busy bringing it to other platforms. Huge shoutout to these contributors 🎊🚀!

| Platform      | Repo                                               | Author                                       |
| ------------- | -------------------------------------------------- | -------------------------------------------- |
| Windows 10/11 | [ThukiWin](https://github.com/ayzekhdawy/thukiwin) | [@ayzekhdawy](https://github.com/ayzekhdawy) |
| Windows 10/11 | [Mate](https://github.com/M31i55a/windowsMate-Thuki) | [@M31i55a](https://github.com/M31i55a) |

> Each port is independently maintained by its author. For issues or questions about a specific port, head to that repo directly.

## Author

Reach out to [Logan](https://x.com/quiet_node) on X with questions or feedback.

## What's next for Thuki

Thuki is just getting started. Here's where it's headed:

### Secretary Superpowers

The big leap: from answering questions to taking action.

- **Tool integrations via [MCP](https://modelcontextprotocol.io/):** connect Thuki to Gmail, Slack, Discord, Google Calendar, and any other MCP-compatible service; ask it to draft a reply, summarize a thread, or schedule a meeting without ever leaving your current app

### Better AI Control

More flexibility over the model powering Thuki.

- **Multiple provider support:** opt in to OpenAI, Anthropic, or any OpenAI-compatible endpoint as an alternative to local Ollama
- **Custom activation shortcut:** change the double-tap trigger to any key or combo you prefer

### Richer Context

Give Thuki more to work with.

- **Voice input:** dictate your question instead of typing
- **Auto-capture screen context:** activate Thuki and have it automatically read the active window or selected region as context (partial: `/screen` captures the full screen today; targeted region capture is next)
- **File and document drop:** drag a PDF or text file directly into Thuki as context for your question

---

Have a feature idea? [Open an issue](https://github.com/quiet-node/thuki/issues) and let's talk about it.

## License

Copyright 2026 Logan Nguyen. Licensed under the [Apache License, Version 2.0](LICENSE).
