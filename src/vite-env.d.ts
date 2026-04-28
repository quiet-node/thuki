/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_QUOTE_MAX_DISPLAY_LINES: string | undefined;
  readonly VITE_QUOTE_MAX_DISPLAY_CHARS: string | undefined;
  readonly VITE_QUOTE_MAX_CONTEXT_LENGTH: string | undefined;
  /** Full git commit SHA injected by CI at build time. Absent in local dev and stable release builds. */
  readonly VITE_GIT_COMMIT_SHA: string | undefined;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
