/* v8 ignore file -- type-only declarations, no runtime code */

/**
 * Snapshot of model picker state returned by the Rust
 * `get_model_picker_state` Tauri command.
 *
 * - `active` is the currently selected Ollama model name, or `null` when
 *   nothing is installed and nothing is persisted. The user must pick a
 *   model from the in-app picker before any chat request can be issued.
 * - `all` is the full list of locally installed Ollama model names, in the
 *   order the backend chose to surface them (typically matches `ollama list`).
 */
export interface ModelPickerState {
  /** The currently active Ollama model name, or null when none is selected. */
  active: string | null;
  /** All locally installed Ollama model names available for selection. */
  all: string[];
  /**
   * Friendly display name per model id, for built-in models whose ids are the
   * raw "repo:file.gguf" slug (e.g. "...:Qwen3.5-9B-Q4_K_M.gguf" -> "Qwen3.5
   * 9B"). Sparse: omitted/absent ids fall back to rendering the id verbatim,
   * which is already clean for Ollama and OpenAI providers.
   */
  displayNames?: Record<string, string>;
  /**
   * Whether the Rust backend successfully reached the local Ollama daemon
   * during the last picker fetch. False when `/api/tags` errored (connection
   * refused, timeout, DNS failure, port closed). The frontend uses this to
   * distinguish "Ollama is down" from "Ollama is up but has no models" and
   * to pick the correct recovery copy in `CapabilityMismatchStrip`.
   */
  ollamaReachable: boolean;
}

/**
 * Per-model capability flags returned by the Rust `get_model_capabilities`
 * Tauri command. Mirrors the `Capabilities` struct in `src-tauri/src/models.rs`.
 */
export interface ModelCapabilities {
  vision: boolean;
  thinking: boolean;
  /**
   * Whether the model's reasoning cannot be turned off (it always reasons,
   * e.g. gpt-oss/Harmony, DeepSeek-R1). The picker badges such models so the
   * user is not surprised by the latency; `/think` is a no-op for them.
   * The backend always sends it; optional here so consumers treat a missing
   * value as "not always" and read it as `reasoningAlways === true`.
   */
  reasoningAlways?: boolean;
  /**
   * Maximum number of images the model accepts in a single request, when
   * known. `null` (or omitted) means Thuki has no architecture-specific
   * cap and trusts Ollama's runner as the final authority. Today this is
   * set to `1` for `mllama`-family models (e.g. llama3.2-vision) which
   * reject multi-image requests with HTTP 500.
   */
  maxImages?: number | null;
}

/**
 * Map of model slug to its capabilities. Built from the Rust command's
 * `HashMap<String, Capabilities>` payload.
 *
 * Modelled as `Partial<Record<...>>` so that lookups on unknown slugs
 * yield `undefined` instead of being silently typed as `ModelCapabilities`.
 * Every consumer is forced to handle the missing-metadata case.
 */
export type ModelCapabilitiesMap = Partial<Record<string, ModelCapabilities>>;
