<h1 align="center">
  Thuki - WIP
</h1>

<p align="center">
  <a href="https://www.thuki.app/" target="_blank" rel="noopener noreferrer"><img src="public/thuki-logo.png" alt="Thuki: a private, local AI secretary overlay for macOS" width="300" /></a>
</p>

<!-- <p align="center">
  <a href="https://www.producthunt.com/products/thuki?embed=true&amp;utm_source=badge-featured&amp;utm_medium=badge&amp;utm_campaign=badge-thuki" target="_blank" rel="noopener noreferrer"><img alt="Thuki  - Floating AI for macOS. Free &amp; Local. No cloud, no API keys. | Product Hunt" width="250" height="54" src="https://api.producthunt.com/widgets/embed-image/v1/featured.svg?post_id=1122707&amp;theme=light&amp;t=1776150241085">  </a>
</p> -->

<p align="center">
  A floating AI secretary for macOS. Double-tap Control to summon a spotlight-style overlay anywhere, even over fullscreen apps. It runs entirely on your Mac with its own built-in engine: private, with no cloud and no API keys.
</p>

<p align="center">
  <strong>Free and open source.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-beta-yellow.svg" alt="Beta" />
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License" /></a>
  <a href="https://www.thuki.app/" target="_blank" rel="noopener noreferrer"><img src="https://img.shields.io/badge/thuki.app-000000?style=flat" alt="thuki.app" /></a>
  <a href="https://github.com/quiet-node/thuki/actions/workflows/pr-pipeline.yml"><img src="https://github.com/quiet-node/thuki/actions/workflows/pr-pipeline.yml/badge.svg" alt="CI" /></a>
  <img src="https://img.shields.io/badge/platform-macOS-lightgrey.svg" alt="Platform: macOS" />
</p>

<p align="center">
<a href="https://www.star-history.com/?repos=quiet-node%2Fthuki">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=quiet-node/thuki&type=date&theme=dark&legend=top-left&sealed_token=n4pSu15VN2npOLh06OaRfRzWUMpaCBX446cKYO-BNCqIc71n6g2_cxC_id6bq9SbVuKo46mBt2N0fVgh4_R28HCyyZbbKSR0sn30p_q4m7gdHTF8K_llDAN25aon3EBR9BS0akE3xlTTszNHObrsfM7b81TDt2zBhky23pay4IBeObvsSc_4ucZStoY9" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=quiet-node/thuki&type=date&legend=top-left&sealed_token=n4pSu15VN2npOLh06OaRfRzWUMpaCBX446cKYO-BNCqIc71n6g2_cxC_id6bq9SbVuKo46mBt2N0fVgh4_R28HCyyZbbKSR0sn30p_q4m7gdHTF8K_llDAN25aon3EBR9BS0akE3xlTTszNHObrsfM7b81TDt2zBhky23pay4IBeObvsSc_4ucZStoY9" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=quiet-node/thuki&type=date&legend=top-left&sealed_token=n4pSu15VN2npOLh06OaRfRzWUMpaCBX446cKYO-BNCqIc71n6g2_cxC_id6bq9SbVuKo46mBt2N0fVgh4_R28HCyyZbbKSR0sn30p_q4m7gdHTF8K_llDAN25aon3EBR9BS0akE3xlTTszNHObrsfM7b81TDt2zBhky23pay4IBeObvsSc_4ucZStoY9" />
 </picture>
</a>
</p>

---

Thuki (thư kí, Vietnamese for secretary) is a lightweight macOS overlay powered by local AI models running entirely on your own machine, built for quick, uninterrupted asks without ever leaving what you're doing.

## Install on macOS

Install the app, pick a model during onboarding, and start asking.

> **Requirements:** macOS 13.4 (Ventura) or later, on Apple Silicon (M1/M2/M3/M4/M5).

### Recommended: one-line install

```bash
curl -fsSL https://thuki.app/install.sh | sh
```

This downloads the latest `Thuki.dmg` over HTTPS, verifies its RSA-4096 signature with the `openssl` already on your Mac, and installs it to `/Applications`. Because the download arrives without a quarantine flag, Thuki opens cleanly: no Gatekeeper "Apple could not verify" prompt and no manual `xattr` step.

#### Nightly (separate app, side by side with stable)

```bash
curl -fsSL https://thuki.app/install.sh | THUKI_CHANNEL=nightly sh
```

Same script URL; the `THUKI_CHANNEL=nightly` env (or `sh -s -- --nightly`) installs **Thuki Nightly.app** beside stable `Thuki.app`. Nightly uses a different bundle id (`com.quietnode.thuki.nightly`), so data, Keychain secrets, and TCC grants stay separate. Not for production. Do not run both at once: they both claim double-tap Control and will fight over the hotkey.

Want to read the script before running it? Visiting [thuki.app/install.sh](https://thuki.app/install.sh) downloads it; open the saved file in a text editor to review it first. Or read it in the terminal without saving anything:

```bash
curl -fsSL https://thuki.app/install.sh | less
```

<details>
<summary><strong>Manual install (download the DMG)</strong></summary>

Prefer to download by hand? Grab the DMG and clear the quarantine flag yourself.

1. Download `Thuki.dmg` from the [latest stable release](https://github.com/quiet-node/thuki/releases/latest), or grab the bleeding-edge build from the [`nightly`](https://github.com/quiet-node/thuki/releases/tag/nightly) channel, rebuilt automatically from `main`.
2. Double-click `Thuki.dmg` to open it, then drag the app onto the `Applications` folder shortcut. Stable ships as `Thuki.app`; nightly as `Thuki Nightly.app`.
3. Eject the disk image (drag it to Trash in the Finder sidebar, or right-click and choose Eject).
4. **Before opening for the first time**, run this command in Terminal (use the path that matches the channel you installed):

   ```bash
   # stable
   xattr -rd com.apple.quarantine /Applications/Thuki.app

   # nightly
   xattr -rd com.apple.quarantine "/Applications/Thuki Nightly.app"
   ```

   > **Why is this needed?** Thuki ships outside the Mac App Store, so Gatekeeper blocks it until this one-time command clears the quarantine flag. It is safe and [documented by Apple](https://support.apple.com/en-us/102445). The one-line installer does this for you.

5. Open the app. It will appear in your menu bar.

</details>

<details>
<summary><strong>Build from source</strong></summary>

**Prerequisites:** [Bun](https://bun.sh) and [Rust](https://rustup.rs)

```bash
# Clone and install dependencies
git clone https://github.com/quiet-node/thuki.git
cd thuki
bun install

# Launch in development mode (hot-reload frontend)
bun run dev
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development setup guide.

To produce a standalone app instead of running the dev server, build it and open the bundle directly:

```bash
bun run build:all
open src-tauri/target/release/bundle/macos/Thuki.app
```

</details>

> **First launch:** macOS will ask for two permissions. **Accessibility** is required for the global keyboard shortcut that lets you summon Thuki from any app. **Screen Recording** is required for the `/screen` command and the screenshot button. Grant both once; they persist across restarts.

> **Pick a model:** onboarding offers a curated set of starter models sized for different Macs and downloads your pick right inside the app. Model files are large (roughly 2-9 GB), so the first download can take several minutes; you only do it once. Add more models anytime, including any GGUF from Hugging Face, from Settings → Models → Discover.

## Why Thuki?

**Most local AI tools are a place you go. Thuki is a key you press.**

- **It arrives knowing what you were looking at.** Highlight text in any app, double-tap Control <kbd>⌃</kbd>, and your selection is already quoted. One gesture, no copy-paste round trip.
- **Nothing to install, pay, or configure first.** Thuki ships its own llama.cpp engine and browses Hugging Face in-app, so any open model is a click away. No terminal, no separate server, no API key, no license fee.
- **It sees your screen, not just your clipboard.** `/screen` attaches your whole desktop as visual context for any vision-capable model.
- **Free and open source, running only on your machine.** No accounts, no per-query cost, no telemetry. Your conversations stay in a local database and nowhere else.
- **Works offline.** Once a model is downloaded, inference needs no connection. Only downloads and web search use the network, and Auto search is one toggle from off.

## Features

<table>
<tr>
<td width="33%"><b>Always one keystroke away</b><br><sub>Double-tap Control from any app, even fullscreen.</sub><br><video src="https://github.com/user-attachments/assets/632d1af8-af83-4c46-8432-713054df8a21"></video></td>
<td width="33%"><b>Highlight, then ask</b><br><sub>Select text, double-tap, it arrives as a quote.</sub><br><video src="https://github.com/user-attachments/assets/1fcb8994-5dbb-42ca-9913-1001e2f1ac15"></video></td>
<td width="33%"><b>Capture your screen</b><br><sub><code>/screen</code> attaches your whole desktop as context.</sub><br><video src="https://github.com/user-attachments/assets/d2400c9d-d99b-4460-aa7c-9d5322c7236b"></video></td>
</tr>
<tr>
<td width="33%"><b>Built-in web search</b><br><sub>Keyless and cited. <code>/search</code> forces a lookup.</sub><br><video src="https://github.com/user-attachments/assets/328d2ae5-5805-4137-b5cc-b895d2375915"></video></td>
<td width="33%"><b>On-device model library</b><br><sub>Download any GGUF, switch from the ask bar.</sub><br><video src="https://github.com/user-attachments/assets/bfffd639-6145-4893-a221-8cefdaafcb34"></video></td>
<td width="33%"><b>Just a slash, zero menus</b><br><sub>Every task is a verb. No dropdowns, no settings.</sub><br><img src="docs/assets/slash-commands.png" alt="Thuki slash commands" width="100%"></td>
</tr>
</table>

## Models & providers

Thuki runs models through a provider. The built-in engine is the default; Ollama is there if you'd rather bring your own.

### Built-in engine (default)

A bundled llama.cpp `llama-server` that Thuki spawns, supervises, and shuts down for you. Download GGUF models such as Llama, Gemma, and Qwen from the Hugging Face Hub right inside the app, then switch between them from the ask bar. No accounts, no API keys, no cost per query.

### Other supported providers

Thuki can also run inference through an external provider instead of the built-in engine.

- **Ollama.** Prefer your own [Ollama](https://ollama.com) install? Switch to it anytime from Settings.
- **Your own OpenAI-compatible server (coming soon).** Support for pointing Thuki at any OpenAI-compatible endpoint you run yourself (a local or self-hosted server) is on the [roadmap](#roadmap).

See [docs/models-and-providers.md](docs/models-and-providers.md) for the full model library and provider guide.

## Privacy

Inference runs on-device, so your prompts, context, and replies never leave your Mac. There is no Thuki account, no API key, no cloud backend, and no telemetry. Conversation history lives in a local SQLite database on your machine; delete a conversation and it is gone. Outbound network use is limited to model downloads from Hugging Face and web search (Auto search and/or `/search`); both search paths can be turned off or avoided. See [docs/privacy.md](docs/privacy.md).

## Architecture & security

Tauri v2 (Rust backend, React and TypeScript frontend) with a bundled llama.cpp `llama-server` engine bound to `127.0.0.1` and killed on quit. The pinned engine release is sha256-verified at build time and every model download is checked against a pinned Hugging Face revision. See [docs/models-and-providers.md](docs/models-and-providers.md) for engine internals and [SECURITY.md](SECURITY.md) for the full security posture.

## Configuration

Thuki works on sensible defaults out of the box. Tweak anything from the in-app Settings panel (open it from the menu-bar icon) or by editing the TOML file at `~/Library/Application Support/com.quietnode.thuki/config.toml`; both write to the same place.

See [docs/configurations.md](docs/configurations.md) for the full schema, [docs/commands.md](docs/commands.md) for the slash command reference, [docs/built-in-web-search.md](docs/built-in-web-search.md) for the built-in search design handbook, and [docs/troubleshooting.md](docs/troubleshooting.md) when something goes wrong.

## Contributing

Contributions are welcome! Read [CONTRIBUTING.md](CONTRIBUTING.md) to get started, and follow the [Code of Conduct](CODE_OF_CONDUCT.md).

Community-maintained Windows ports: [ThukiWin](https://github.com/ayzekhdawy/thukiwin) and [Mate](https://github.com/M31i55a/windowsMate-Thuki).

## Roadmap

MCP tool integrations (Gmail, Slack, Calendar), voice input, meeting notes, task automation, and bring-your-own OpenAI-compatible server. Have an idea? [Open an issue](https://github.com/quiet-node/thuki/issues).

## Founder note

Hey, Logan here. I'm building Thuki around how people actually use it, so if you have feedback, an idea, or just want to say hi, [reach out on X](https://x.com/quiet_node). Or [leave your email](https://thuki.app/subscribe) and I'll reach out personally. I read everything.

## License

Copyright 2026 Logan Nguyen. Licensed under the [Apache License, Version 2.0](LICENSE).
