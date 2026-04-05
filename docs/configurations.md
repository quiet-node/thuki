# Configurations

Thuki uses environment variables for runtime configuration. Vite loads these from `.env` files at build/dev time and exposes variables prefixed with `VITE_` to the frontend via `import.meta.env`.

## Setup

```bash
cp .env.example .env
```

Edit `.env` to override any defaults. Changes take effect on the next `bun run dev` or `bun run build:all`.

> `.env` is gitignored. `.env.example` is committed as the reference template.

## Configuration Reference

### Quote Display

Controls how selected-text quotes are displayed in the AskBar preview and chat bubbles, and how much context is forwarded to the LLM.

| Variable | Description | Default | Type |
| :--- | :--- | :--- | :--- |
| `VITE_QUOTE_MAX_DISPLAY_LINES` | Maximum number of lines shown in the quote preview. Lines beyond this limit are truncated with `...`. Empty lines in the selection are skipped and do not count toward this limit. | `4` | Positive integer |
| `VITE_QUOTE_MAX_DISPLAY_CHARS` | Maximum total characters shown in the quote preview. If a line would push the total past this limit, it is truncated mid-line with `...`. | `300` | Positive integer |
| `VITE_QUOTE_MAX_CONTEXT_LENGTH` | Maximum length (in characters) of selected context text included in the prompt sent to Ollama. This is a security and performance cap; selections longer than this are silently truncated before reaching the LLM. | `4096` | Positive integer |

### System Prompt

Controls the system prompt prepended to every conversation sent to Ollama.

| Variable | Description | Default |
| :--- | :--- | :--- |
| `THUKI_SYSTEM_PROMPT` | Custom system prompt for all conversations. Set to an empty string to use the built-in default. | `"You are Thuki, a concise desktop secretary. Keep responses short and direct."` |

### Validation Rules

All configuration values are validated at startup via `src/config/index.ts`:

- **Missing or empty** values fall back to the default.
- **Non-numeric** values (e.g., `abc`) fall back to the default.
- **Zero or negative** values fall back to the default.
- **Decimal** values are floored to the nearest integer (e.g., `5.7` becomes `5`).
- **Infinity** falls back to the default.

### File Precedence

Vite loads `.env` files in the following order (later files override earlier ones):

| File | Purpose | Committed |
| :--- | :--- | :--- |
| `.env.example` | Reference template with documented defaults | Yes |
| `.env` | Local configuration | No (gitignored) |
| `.env.local` | Local overrides (highest priority) | No (gitignored via `*.local`) |
| `.env.development` | Dev-only overrides (loaded when `bun run dev`) | Optional |
| `.env.production` | Prod-only overrides (loaded when `bun run build:all`) | Optional |
