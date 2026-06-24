# Models and Providers

Thuki ships its own AI engine, so it works the moment you install it: no account, no API key, no separate server to run. This guide explains where your models live, how to add more, and how to switch between the built-in engine and an optional Ollama install.

> macOS only. Thuki is a Mac app and assumes Apple Silicon (M1/M2/M3/M4/M5).
>
> See [thuki.app](https://www.thuki.app/) for project info, downloads, and documentation.

## The built-in engine

On a fresh install Thuki runs the **built-in engine**: a bundled `llama.cpp` server that Thuki starts, supervises, and shuts down for you. Everything runs on your Mac, and nothing leaves it. During onboarding Thuki downloads one starter model so you can begin right away; you add more whenever you like.

Models for the built-in engine are GGUF files Thuki downloads and stores itself. You never edit a config file or manage a server to use them.

## Settings → Models

Open Settings from the Thuki icon in your menu bar: right-click it and choose **Settings…**. Then select **Models**, which has three tabs.

### Library

Your installed models. Each row shows the model name, its capability pills, and a `size · context · maker · quant` sub-line. From the row menu you can:

- **Set as active** so the next chat uses it.
- **Reveal** the downloaded file on disk.
- **Delete** it to reclaim space.

### Discover

Where you find and download new models. Two sub-tabs:

- **Staff picks** — a small, vetted catalog grouped by what you want to do (everyday chat, compact and fast, deep reasoning). A safe place to start.
- **Browse all** — a live search of GGUF models on the Hugging Face Hub. Anything you find downloads straight into your Library. Quality and safety vary on the open Hub, so research a model before you run it.

Each model shows an approximate RAM-fit hint for your Mac and its trained context window, so you can pick one that fits.

### Providers

Manage which engine answers your chats. The active provider sits at the top as a hero card; the other appears as a compact row you can switch to. Below them is a shared **Generation** section that applies to whichever provider is active:

- **Context window** — how much conversation the model can see at once. See [Tuning the Context Window](./tuning-context-window.md).
- **Keep Warm** — how long the active model stays resident in memory between messages, so you skip the cold-load wait. Set it to `-1` to keep it loaded until you unload it yourself, or use **Unload now** to free memory immediately.
- **System prompt** — the persona and instructions sent at the start of every conversation.

## Capabilities

Each model advertises what it can do with passive badges:

- **Vision** — the model accepts images. Paste or drag an image into the chat, or capture one with `/screen`, and ask about it. (Text-only models can still read images through on-device OCR; see [OCR commands](./ocr-commands.md).)
- **Reasoning** — the model can think through a problem before answering. Trigger it per message with `/think`; a collapsible Reasoning block appears above the answer. Models marked **Always thinks** reason on every reply.

You can switch models mid-conversation. Thuki adapts the history to whatever the new model supports.

## Using your own Ollama install (optional)

Prefer to run models through [Ollama](https://ollama.com)? Switch to the Ollama provider in **Settings → Models → Providers**. Install Ollama, pull a model (`ollama pull <slug>`), and Thuki reaches it at `http://127.0.0.1:11434` by default (you can point it at another machine). Switching providers frees the one you switched away from, so only one model is resident at a time.
