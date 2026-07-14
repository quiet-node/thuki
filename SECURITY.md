# Security Policy

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report vulnerabilities privately via [GitHub Security Advisories](https://github.com/quiet-node/thuki/security/advisories/new). This keeps the details confidential until a fix is ready.

We will acknowledge your report within **48 hours** and aim to release a fix within **14 days** for critical issues, depending on severity and complexity.

## Scope

Thuki runs entirely on your local machine: no server, no cloud backend, no telemetry. Inference happens on-device through the bundled engine (or your own Ollama install). Two things reach the network: downloading a model from the Hugging Face Hub, and web search, either triggered automatically by **Auto search** (Settings → Behavior, on by default) when a plain turn needs live facts, or forced on demand with `/search`. Both can be turned off in Settings. The attack surface is limited to:

- The Tauri IPC boundary between the frontend and Rust backend
- The macOS Accessibility API integration that captures selected text and screen bounds at activation (`context.rs`)
- Screen capture via CoreGraphics (`screenshot.rs`), covering both interactive `screencapture` selection and full-screen `CGWindowListCreateImage`
- The bundled llama.cpp `llama-server` sidecar, which binds to `127.0.0.1` only with its web UI disabled, so nothing off your machine can reach it
- Parsing of downloaded GGUF model metadata, which is bounded and panic-safe against malformed or hostile files
- Model downloads from the Hugging Face Hub: provenance comes from pinned repo revisions, while the sha256 check is an integrity guard (truncation, bit rot, resume corruption), not a provenance control
- The local SQLite database storing conversation history
- Image processing via the `image` crate
- Built-in web search egress (`src-tauri/src/websearch/` via `src-tauri/src/net/`): keyless engine and vertical fetches, page download, SSRF default-deny for private/link-local targets, redirect re-check, and response size caps. Untrusted page text is nonce-fenced before it reaches the writer model. See [docs/privacy.md](docs/privacy.md) and [docs/configurations.md](docs/configurations.md) (Built-in web search).

## Supported Versions

We support the latest release only. Please verify you are on the latest version before reporting.
