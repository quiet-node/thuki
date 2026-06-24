# Privacy

Thuki is built so your conversations stay on your Mac. This page explains exactly what runs locally, the few things that touch the network, and why.

> macOS only. See [thuki.app](https://www.thuki.app/) for project info and downloads.

## Local by default

Inference runs on-device. The bundled built-in engine (or your own Ollama install, if you choose it) generates every answer on your Mac. There is no Thuki account, no API key, no cloud backend, and no telemetry. Your prompts, your context, and the model's replies never leave the machine.

Your conversation history lives in a local SQLite database on your Mac and nowhere else. Delete a conversation and it is gone.

## What touches the network, and only when you ask

Thuki makes no background calls. The only outbound requests are ones you initiate:

- **Downloading a model.** When you install a model from Discover, Thuki fetches it from the Hugging Face Hub. After the download, inference is fully offline.
- **`/search`.** The optional agentic-search pipeline fetches web pages to answer a `/search` query. It runs against local services you start yourself; if you never use `/search`, nothing is fetched. See [Agentic search](./agentic-search.md).

Everything else, including reading your selected text and capturing your screen, happens entirely on-device.

## Permissions

Thuki asks for two macOS permissions, both local:

- **Accessibility** lets Thuki detect the double-tap Control shortcut and read the text you have highlighted so it can pre-fill a quote. It is never sent anywhere.
- **Screen Recording** lets `/screen` and the screenshot button capture your display. Captures are processed on-device and attached only to the message you send.

## Where models come from

Built-in models are pinned to exact Hugging Face repo revisions, and each file is checked against a known sha256 hash after download. The hash is an integrity check (it catches a truncated or corrupted download), and the pinned revision is what fixes exactly which file you get. Models from **Browse all** come straight from the open Hub, so review a model's source before you run it.

## Reporting a concern

Found something that looks like a privacy or security issue? See [SECURITY.md](../SECURITY.md) for how to report it privately.
