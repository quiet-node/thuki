/**
 * Compile-time, dev-only feature flags for the Settings UI.
 *
 * These are resolved from `import.meta.env` at build time, so a flag that is
 * `false` in a production build lets Vite tree-shake the gated affordance out
 * of the shipped bundle entirely.
 */

/**
 * Whether the OpenAI-compatible provider KIND is exposed in the Settings UI.
 *
 * Thuki ships local-only (the bundled built-in engine plus local or remote
 * Ollama). The `openai` backend (the shared `/v1` client, Keychain storage,
 * the routing arm, the config kind) stays live: this flag gates only the
 * user-facing affordances that let someone create or manage an `openai`
 * provider, so no end user can reach one in a shipped build.
 *
 * Off by default. Developers opt in at build time by setting
 * `VITE_ENABLE_OPENAI_PROVIDER=true`; any other value (including unset) is
 * off.
 */
export const OPENAI_PROVIDER_ENABLED =
  import.meta.env.VITE_ENABLE_OPENAI_PROVIDER === 'true';
