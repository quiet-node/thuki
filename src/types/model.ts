/* v8 ignore file -- type-only declarations, no runtime code */

/**
 * Snapshot of model picker state returned by the Rust
 * `get_model_picker_state` Tauri command.
 *
 * - `active` is the currently selected Ollama model name. Never empty once
 *   the backend has completed startup seeding.
 * - `all` is the full list of locally installed Ollama model names, in the
 *   order the backend chose to surface them (typically matches `ollama list`).
 */
export interface ModelPickerState {
  /** The currently active Ollama model name. */
  active: string;
  /** All locally installed Ollama model names available for selection. */
  all: string[];
}

/**
 * Per-model capability flags returned by the Rust `get_model_capabilities`
 * Tauri command. Mirrors the `Capabilities` struct in `src-tauri/src/models.rs`.
 */
export interface ModelCapabilities {
  vision: boolean;
  thinking: boolean;
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
