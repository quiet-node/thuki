# /extract: OCR Text Extraction

`/extract` reads text from screenshots and images using the macOS Vision framework. It runs entirely on-device with no network calls, no model loading, and no LLM round-trip. Results appear in under a second for typical screenshots.

## What is OCR?

OCR (Optical Character Recognition) is the process of detecting and reading text in images. Given a pixel grid, the OCR engine identifies character shapes, groups them into words and lines, and returns the recognized text. The result is machine-readable text that can be copied, searched, or processed further.

Modern OCR engines (including the one powering `/extract`) are not guessing based on context. They apply trained convolutional neural networks to detect text regions, segment individual characters, and classify each glyph. The output is deterministic for a given image.

## Why no LLM?

Most AI assistants that "read" images send the image to a vision-capable language model. The model describes what it sees, including the text. This works but introduces several costs:

- **Latency:** The model must load (if not already warm), tokenize the image, run a forward pass, and stream tokens back. For a text-only extraction task, this adds 1-10 seconds of overhead.
- **Accuracy:** LLMs can hallucinate or paraphrase text. A vision model asked to "extract text" may still rephrase, correct apparent typos, or drop content it considers noise. OCR engines report what the pixels say, faithfully.
- **Token cost:** Image tokens are expensive. A 1080p screenshot may consume 500-1000 tokens just to encode, before the model writes a single character of output.
- **VRAM:** Running a multimodal model requires a vision-capable Ollama model loaded in GPU memory. Not every setup has one, and loading one takes time.

`/extract` bypasses all of this. It calls `VNRecognizeTextRequest` directly via the macOS Vision framework, which is a compiled CoreML-backed pipeline that runs in milliseconds on CPU. No model, no stream, no round-trip.

## How it works

When you submit `/extract`, Thuki:

1. Collects all attached images (pasted, dragged, or from a combined `/screen /extract` capture).
2. Invokes the Rust backend command `extract_text_command` via the Tauri IPC layer.
3. For each image path, calls the macOS Vision framework (`VNRecognizeTextRequest`) at accuracy level `VNRequestTextRecognitionLevelAccurate`.
4. Collects the recognized text from each `VNRecognizedTextObservation` in document order (top-to-bottom, left-to-right).
5. Joins lines with `\n` per image. If multiple images were provided, results are separated with `\n\n---\n\n`.
6. Returns the raw text with no paraphrasing, no commentary, and no LLM involvement.

If every image is blank (no readable text detected), the response is `[No text detected]`.

### Fallback to Ollama vision model

If Vision OCR fails (e.g., an unsupported image format), Thuki falls back to your active Ollama model only if it has vision capability. The fallback prompt asks the model to extract text verbatim. If no vision model is active, Thuki surfaces an error instead of silently doing nothing.

## Performance

Typical wall-clock times on Apple Silicon:

| Source | Time |
|---|---|
| Single screenshot (1080p) | Under 200ms |
| Four attached images | Under 500ms |
| Combined `/screen /extract` (capture + OCR) | Under 700ms |

These numbers reflect the Vision framework running on the Neural Engine / CPU. There is no warm-up delay, no tokenization, and no streaming. The result is ready as soon as the framework finishes its recognition pass.

By contrast, sending the same screenshot to a vision LLM typically takes 2-10 seconds, depending on model size and whether it is already loaded. For a repeated text-extraction workflow (e.g., capturing terminal errors, reading pricing tables, copying text from PDFs), `/extract` is consistently 10-50x faster.

## Usage patterns

### Basic: extract text from a pasted image

1. Copy any image to your clipboard (screenshot, PDF page, photo).
2. Summon Thuki and paste the image (Command-V or drag-and-drop).
3. Type `/extract` and press Enter.
4. The extracted text appears in the chat, ready to copy.

### Combined: capture screen then extract

```
/screen /extract
```

Takes a screenshot and immediately runs OCR on it. The screen capture and text extraction happen in a single submit. Useful for quickly grabbing terminal output, error messages, or any on-screen text you need as plain text.

### Multiple images

Paste or drag up to 4 images before submitting `/extract`. Each image is processed independently; results appear in order, separated by `---` dividers.

### With an instruction

You can type a message alongside `/extract`, but it has no effect on the OCR output itself (no LLM is involved). The message is preserved in your conversation bubble for context. If you want the LLM to do something with the extracted text afterward, run `/extract` first, then follow up with a separate message.

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

For these cases, the Ollama vision fallback may produce better results since the model uses context and can infer partial characters.

## Technical details

The backend is implemented in `src-tauri/src/ocr.rs` using the `objc2` and `objc2-foundation` crates for safe Objective-C interop, with `objc2-vision` providing the Vision framework bindings.

Key implementation choices:

- **`VNRequestTextRecognitionLevelAccurate`**: the highest accuracy level, which uses a neural language model to correct recognition errors. The alternative (`Fast`) skips the language model and is roughly 3x faster but less accurate. `Accurate` is the right default for a text extraction use case where accuracy matters more than latency that is already imperceptible.
- **Line order**: Vision returns observations sorted by position (top-to-bottom, left-to-right in screen coordinates). This matches the reading order of most western-language documents.
- **No post-processing**: `process_raw_text` only trims whitespace to detect blank results. All content is returned as-is, including special characters, symbols, and code.
- **Coverage exclusion**: The FFI wrapper and Tauri command are excluded from coverage with `#[cfg_attr(coverage_nightly, coverage(off))]` because they require a running display server and Screen Recording permission. The pure logic helpers (`process_raw_text`, `join_ocr_results`) have 100% test coverage.

## Permissions

`/extract` requires no additional permissions beyond what `/screen` already uses when combined with it. Standalone `/extract` on attached images requires no special permissions: Vision OCR operates on local file paths and does not trigger Screen Recording.

Combined `/screen /extract` requires the same Screen Recording permission as plain `/screen`.
