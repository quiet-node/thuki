# OCR-Supported Commands

Thuki has a class of slash commands that read text out of attached images locally using the **macOS Vision framework**. No network calls, no LLM round-trip, and crucially: **no vision-capable model required**. The OCR engine extracts text on-device; the resulting text is what the active model sees.

## Which commands use OCR

| Command | What it does with OCR'd text |
|---|---|
| `/extract` | Returns the raw text verbatim. No LLM call. |
| `/tldr` | Summarizes the extracted text in 1-3 sentences. |
| `/translate` | Translates the extracted text to a target language. |
| `/rewrite` | Rewrites the extracted text for clarity. |
| `/refine` | Fixes grammar and spelling in the extracted text. |
| `/bullets` | Turns the extracted text into a bullet list. |
| `/todos` | Pulls action items out of the extracted text. |
| `/explain` | Explains the extracted text in plain language. |

Every command above accepts attached images, a `/screen` capture, or both as its input source. The image bytes themselves are never sent to the model.

## Why they work on text-only models

A vision model is normally required when an image is part of the request. Models like `llama3.2:3b` reject or hallucinate when handed an image directly. The OCR-supported commands sidestep this entirely:

1. Vision framework runs OCR on the image locally.
2. The recognized text replaces the image in the prompt.
3. The active model receives plain text only.

This means you can use `llama3.2:3b`, `qwen2.5:7b`, `gemma:7b`, or any other text-only model on image inputs as long as you go through one of the OCR-supported commands. The capability strip will guide you toward this when an image is attached.

For plain submits (no slash command) and `/screen` alone, a vision-capable model is still required because the image bytes go directly to the model.

## What is OCR?

OCR (Optical Character Recognition) detects and reads text in images. Given a pixel grid, the engine identifies character shapes, groups them into words and lines, and returns the recognized text. The result is machine-readable text that can be copied, searched, or processed further.

Modern OCR engines (including the one powering these commands) are not guessing based on context. They apply trained convolutional neural networks to detect text regions, segment individual characters, and classify each glyph. The output is deterministic for a given image.

## Why no LLM for the OCR step?

Most AI assistants that "read" images send the image to a vision-capable language model. The model describes what it sees, including the text. This works but introduces several costs:

- **Latency:** The model must load (if not already warm), tokenize the image, run a forward pass, and stream tokens back. For a text-only extraction task, this adds 1-10 seconds of overhead.
- **Accuracy:** LLMs can hallucinate or paraphrase text. A vision model asked to "extract text" may still rephrase, correct apparent typos, or drop content it considers noise. OCR engines report what the pixels say, faithfully.
- **Token cost:** Image tokens are expensive. A 1080p screenshot may consume 500-1000 tokens just to encode, before the model writes a single character of output.
- **VRAM:** Running a multimodal model requires a vision-capable Ollama model loaded in GPU memory. Not every setup has one, and loading one takes time.

The OCR commands bypass all of this. They call `VNRecognizeTextRequest` directly via the macOS Vision framework, which is a compiled CoreML-backed pipeline that runs in milliseconds on CPU. No model, no stream, no round-trip for the OCR step. The utility commands (`/tldr`, `/translate`, etc.) still call the model for the post-OCR work, but only with plain text.

## How it works

When you submit any OCR-supported command, Thuki:

1. Waits for every attached image to finish backend processing (pending-image gate).
2. Collects all attached images plus any fresh `/screen` capture.
3. Invokes the Rust backend command `extract_text_command` via the Tauri IPC layer.
4. For each image path, calls the macOS Vision framework (`VNRecognizeTextRequest`) at accuracy level `VNRequestTextRecognitionLevelAccurate`.
5. Collects the recognized text from each `VNRecognizedTextObservation` in document order (top-to-bottom, left-to-right).
6. Joins lines with `\n` per image. If multiple images were provided, results are separated with `\n\n---\n\n`.
7. For `/extract`: returns the raw text verbatim. For utility commands: fills `$INPUT` in the prompt template with the OCR result, then calls the active model.

If every image is blank (no readable text detected), `/extract` returns `[No text detected]`. Utility commands surface a friendly error so the model is not asked to summarize an empty string.

### Fallback to Ollama vision model (/extract only)

If Vision OCR fails on `/extract` (e.g., an unsupported image format), Thuki falls back to your active Ollama model only if it has vision capability. The fallback prompt asks the model to extract text verbatim. If no vision model is active, Thuki surfaces an error instead of silently doing nothing. Utility commands do not currently fall back; their OCR failure surfaces as a capture error.

## Performance

Typical wall-clock times on Apple Silicon (OCR step only):

| Source | Time |
|---|---|
| Single screenshot (1080p) | Under 200ms |
| Four attached images | Under 500ms |
| Combined `/screen /extract` (capture + OCR) | Under 700ms |

These numbers reflect the Vision framework running on the Neural Engine / CPU. There is no warm-up delay, no tokenization, and no streaming. The OCR result is ready as soon as the framework finishes its recognition pass.

By contrast, sending the same screenshot to a vision LLM typically takes 2-10 seconds, depending on model size and whether it is already loaded. For a repeated text-extraction workflow (e.g., capturing terminal errors, reading pricing tables, copying text from PDFs), the OCR-supported commands are consistently 10-50x faster for the OCR step.

## Usage patterns

### Extract raw text

1. Copy any image to your clipboard.
2. Summon Thuki and paste the image.
3. Type `/extract` and press Enter.
4. Raw text appears in the chat, ready to copy.

### Capture screen then extract

```
/screen /extract
```

Takes a screenshot and immediately runs OCR on it. Useful for grabbing terminal output, error messages, or any on-screen text you need as plain text.

### Summarize a screenshot on a text-only model

1. Switch to `llama3.2:3b` (or any text-only model) in the model picker.
2. Paste a screenshot.
3. Type `/tldr` and press Enter.
4. Thuki OCRs the screenshot locally, then sends only the recognized text to the model, which returns a 1-3 sentence summary.

The same pattern works for `/translate`, `/rewrite`, `/refine`, `/bullets`, `/todos`, and `/explain`.

### Multiple images

Paste or drag up to 4 images before submitting any OCR-supported command. Each image is processed independently; results appear in order, separated by `---` dividers in the `/extract` raw view, or concatenated as the `$INPUT` to the model for utility commands.

## What Vision OCR handles well

- Terminal and IDE output (monospace code, error messages, stack traces)
- App screenshots with standard system fonts
- Web page captures (news articles, documentation, pricing pages)
- Scanned documents with clear print at reasonable resolution
- Spreadsheets and tables with clearly delineated cells
- PDF pages captured via screenshot

## What Vision OCR may struggle with

- Handwritten text (accuracy varies with legibility)
- Rotated or heavily skewed text
- Very small text (under 8pt at normal screen resolution)
- Text overlaid on complex or similarly-colored backgrounds
- Heavily stylized display fonts
- Extreme compression artifacts (high-JPEG-compression screenshots)

For these cases, the Ollama vision fallback (on `/extract`) may produce better results because the model uses context and can infer partial characters. For utility commands, switching to a vision model and re-submitting without the slash command sends the image directly to the model instead.

## Technical details

The backend is implemented in `src-tauri/src/ocr.rs` using the `objc2` and `objc2-foundation` crates for safe Objective-C interop, with `objc2-vision` providing the Vision framework bindings.

Key implementation choices:

- **`VNRequestTextRecognitionLevelAccurate`**: the highest accuracy level, which uses a neural language model to correct recognition errors. The alternative (`Fast`) skips the language model and is roughly 3x faster but less accurate. `Accurate` is the right default for a text extraction use case where accuracy matters more than latency that is already imperceptible.
- **Line order**: Vision returns observations sorted by position (top-to-bottom, left-to-right in screen coordinates). This matches the reading order of most western-language documents.
- **No post-processing**: `process_raw_text` only trims whitespace to detect blank results. All content is returned as-is, including special characters, symbols, and code.
- **Coverage exclusion**: The FFI wrapper and Tauri command are excluded from coverage with `#[cfg_attr(coverage_nightly, coverage(off))]` because they require a running display server and Screen Recording permission. The pure logic helpers (`process_raw_text`, `join_ocr_results`) have 100% test coverage.

## Permissions

OCR-supported commands require no additional permissions when operating on attached images: Vision OCR operates on local file paths and does not trigger Screen Recording.

When combined with `/screen` (e.g., `/screen /extract`, `/screen /tldr`), the same Screen Recording permission as plain `/screen` is required.
