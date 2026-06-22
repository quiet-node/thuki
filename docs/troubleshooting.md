# Troubleshooting

Common issues and how to fix them. If none of these match, open an issue at [github.com/quiet-node/thuki](https://github.com/quiet-node/thuki/issues).

> macOS only. See [thuki.app](https://www.thuki.app/) for project info and downloads.

## Models and the engine

**"Unsupported model."** The bundled engine does not recognize this model's architecture yet. This usually means a brand-new or experimental model format. Pick a different model (the **Staff picks** in Discover are all verified to run), and check back later as engine support expands with updates.

**The first reply is slow.** The model has to load from disk into memory the first time you use it (the cold load). Turn on **Keep Warm** in Settings → Models → Providers to hold the active model in memory between messages so later replies start instantly.

**Replies suddenly got slow, or the Mac feels sluggish.** Your context window is probably too large for your memory, so the model spilled onto the CPU. Lower the **Context Window** in Settings, or switch to a smaller model. The [context-window tuning guide](./tuning-context-window.md) walks through finding a value that fits.

**"No model downloaded yet" / no model to chat with.** Open **Settings → Models → Discover** and download one. The Staff picks are sized for different Macs and show an approximate RAM-fit hint.

**A download failed or stalled.** Downloads resume where they left off, so try again; Thuki picks up the partial file. A failed checksum means the file arrived corrupted, in which case Thuki discards it and you can re-download cleanly.

## Using Ollama (optional)

**"Ollama isn't running."** You have the Ollama provider selected but the Ollama app or server is not running. Start Ollama and try again, or switch back to the built-in engine in Settings → Models → Providers.

**A model you pulled in Ollama is not in the picker.** Thuki reads Ollama's installed models live. If a model is missing, confirm it finished pulling (`ollama list` in a terminal), then reopen the picker.

## Activation and capture

**Double-tap Control does not open Thuki.** Thuki needs the **Accessibility** permission to detect the shortcut. Open System Settings → Privacy & Security → Accessibility, make sure Thuki is enabled, and toggle it off and on if it was already listed.

**`/screen` or the screenshot button does nothing.** Thuki needs the **Screen Recording** permission. Enable Thuki under System Settings → Privacy & Security → Screen Recording, then relaunch the app.

## Network

**Thuki works offline, except downloads.** Once a model is installed, chatting needs no internet. Only downloading a new model (and the optional `/search`) reaches the network. If a download will not start, check your connection and that you can reach the Hugging Face Hub.
