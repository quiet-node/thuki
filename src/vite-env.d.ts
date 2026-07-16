/// <reference types="vite/client" />

/** Vite `?raw` import of a markdown file as a string. */
declare module '*.md?raw' {
  const content: string;
  export default content;
}

interface ImportMetaEnv {
  readonly VITE_QUOTE_MAX_DISPLAY_LINES: string | undefined;
  readonly VITE_QUOTE_MAX_DISPLAY_CHARS: string | undefined;
  readonly VITE_QUOTE_MAX_CONTEXT_LENGTH: string | undefined;
  /** Full git commit SHA injected by CI at build time. Absent in local dev and stable release builds. */
  readonly VITE_GIT_COMMIT_SHA: string | undefined;
  /**
   * Dev-only opt-in for the OpenAI-compatible provider UI. `"true"` exposes
   * the Settings affordances to create/manage an `openai` provider; any other
   * value (including unset) hides them. Off in shipped builds.
   */
  readonly VITE_ENABLE_OPENAI_PROVIDER: string | undefined;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
