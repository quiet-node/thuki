# Tuning the Context Window for Your Mac

The Context Window slider in Settings goes up to 1 M tokens, but the value that's actually _good_ for your machine depends on your GPU memory and the model you picked. This guide explains what those numbers mean and walks you through finding your sweet spot in about 5 minutes.

> macOS only. Thuki is a Mac app and the steps below assume Apple Silicon (M1/M2/M3/M4/M5).

## Quick vocabulary

A few terms you'll see in this doc and in tools like `ollama ps`:

- **Model weights**: the trained "knowledge" of the model. Fixed size; does not change with your settings. Loaded into memory once.
- **Token**: a chunk of text, roughly ¾ of a word. "Context window in tokens" means the model can see that many word-chunks at once.
- **Context window (`num_ctx`)**: how many tokens the model can see in a single conversation. Bigger window means more conversation history visible to the model.
- **KV cache**: scratch space the model uses to remember the conversation while generating. Grows with the context window. **Doubling `num_ctx` roughly doubles the KV cache.** Model weights stay the same size.
- **GPU**: the chip that runs the math. On Apple Silicon Macs, the GPU is built into the same chip as the CPU.
- **VRAM / "GPU memory"**: the memory the GPU can read directly. On Apple Silicon this is _unified memory_, shared with the CPU; there is no separate VRAM chip. So when we say "Ollama is using 7 GiB of VRAM", we mean it is holding 7 GiB of your unified memory and the GPU has direct access to it.
- **Cold load**: the few seconds it takes to read the model from disk into memory the first time you use it.
- **Keep Warm**: tells Ollama to leave the model in memory after a reply, so the next message skips the cold load.

## What Ollama does behind the scenes

- When you send your first message, Ollama reads your selected model from disk into unified memory. This is the cold load.
- It also allocates the KV cache based on `num_ctx`. A bigger context means a bigger allocation.
- After the reply, Ollama keeps the model in memory for **5 minutes by default** (the `keep_alive` setting), then unloads it. The next request after that pays the cold load again.
- If you set a `num_ctx` larger than the model can actually handle, Ollama silently caps it. Example: you set 1 M, the model maxes out at 128 K, so Ollama uses 128 K. No error, just clamped down.
- If the requested memory exceeds what's available on the GPU, Ollama puts part of the model on the CPU instead. **This is the slow path** and is what we want to avoid.

## The three signals to watch

These are the only signals you need to decide whether your settings are healthy:

1. **Is the model 100% on GPU?** Most important. CPU spill makes inference 5-20× slower.
2. **Is system Memory Pressure green?** Leaves headroom for macOS and your other apps.
3. **Does the GPU actually fire when you generate?** Sanity check that the model is doing real work.

## The 5-minute benchmark recipe

### Step 1 — Pick a starting value

Open Thuki **Settings → Context Window**. Set the slider to **16384** (16K). This is the default and works on most Macs.

### Step 2 — Open Activity Monitor's Memory tab

1. Press `Cmd + Space`, type **Activity Monitor**, press Enter.
2. Click the **Memory** tab at the top of the window.
3. Look at the bottom of the window for the **Memory Pressure** graph (the colored graph in the lower-left). Green is good.
4. Leave this window visible.

<!-- screenshot: Activity Monitor → Memory tab with the Memory Pressure graph circled -->

### Step 3 — Open the GPU History window

GPU History is a separate floating window inside Activity Monitor. To open it:

1. With Activity Monitor focused (click anywhere inside its window first), look at the macOS menu bar at the very top of your screen.
2. Click **Window** in the menu bar (between "View" and "Help").
3. From the dropdown, click **GPU History** (keyboard shortcut: `Cmd + 4`).
4. A small floating window appears showing live GPU activity bars. Drag it next to Activity Monitor.

> If you don't see a "Window" menu in the menu bar, click anywhere in the Activity Monitor window first to focus it, then look at the menu bar again.

<!-- screenshot: Activity Monitor menu bar showing the Window menu open with GPU History highlighted -->

<!-- screenshot: GPU History floating window showing spike pattern during a Thuki message -->

### Step 4 — Open Terminal

1. Press `Cmd + Space`, type **Terminal**, press Enter.
2. Place it next to the other two windows.

### Step 5 — Send a test message

Open Thuki and send your usual kind of question, or paste a long block of text and ask about it. While the reply streams, watch:

- The **GPU History** bars should spike high.
- **Memory Pressure** should stay green.

### Step 6 — Check what Ollama actually did

While the reply is on screen (or right after), run in Terminal:

```bash
ollama ps
```

You'll see something like:

```
NAME         ID            SIZE     PROCESSOR    CONTEXT    UNTIL
gemma4:e2b   7fbdbf8f5e45  7.4 GB   100% GPU     16384      4 minutes from now
```

<!-- screenshot: Terminal output of `ollama ps` for a healthy 100% GPU run -->

What to read:

- `PROCESSOR` must read **`100% GPU`**. If it shows `47%/53% CPU/GPU` (or any split), the model spilled out of unified memory. Too much context for your hardware.
- `SIZE` is the total footprint right now (model weights + KV cache).
- `CONTEXT` shows the actual context length Ollama used, after any silent clamping to your model's trained max.
- `UNTIL` shows when Keep Warm will release the model.

Note the SIZE value. You'll compare it against the next try.

### Step 7 — Bump the context and repeat

Go back to Thuki Settings and double the value (16K → 32K → 64K → ...). Send another test message. Re-run `ollama ps`.

Stop the moment **any** of these happens:

- `PROCESSOR` drops below 100% GPU, **or**
- Memory Pressure turns yellow or red, **or**
- Replies feel sluggish.

### Step 8 — Lock in your sweet spot

Set Thuki to one tier _below_ your last working value for safety margin. Example: 64K worked but `SIZE` was tight against your unified memory total → use 32K. You're done.

## Picking Keep Warm

Keep Warm is the second knob in the same Settings section. It tells Ollama how long to leave the model in memory between messages.

- **`0`** — let Ollama use its 5-minute default. Good baseline.
- **5 to 30 minutes** — good if you use Thuki in bursts every few minutes.
- **`-1`** — always loaded. Only choose this if you have memory headroom and want zero cold-start ever.
- **Unload now** — manual eject when you're done for the day.

## Common results explained

- **"GPU is 0% when Thuki is idle."** Normal. Keep Warm holds the model in memory, but the GPU only fires during generation. Memory residency and active compute are different things.
- **"CPU stays low even during a reply."** Normal. Metal runs the math on the GPU; the CPU only orchestrates.
- **"I set 1 M but `ollama ps` shows 128 K."** Normal. Ollama caps at the model's trained max and silently clamps down.
- **"Model unloads on its own."** Either your Keep Warm timer expired or something else (you, or another tool) ran `ollama stop`.
- **"Inference suddenly got slow."** Check `ollama ps` for a `CPU/GPU` split. You've spilled out of unified memory. Lower `num_ctx` or pick a smaller model.

## Going deeper

If you want raw machine-readable numbers, the same data plus a few extra fields is available from the Ollama HTTP API:

```bash
curl -s http://127.0.0.1:11434/api/ps | jq
```

Useful extra fields not shown by `ollama ps`:

- `size_vram` — bytes the GPU is actually addressing (vs `size`, which includes any CPU portion when the model spilled).
- `expires_at` — exact ISO timestamp when Keep Warm will release the model.
- `digest` — content hash of the loaded model file.
