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
