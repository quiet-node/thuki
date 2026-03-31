# Thuki

The context-aware floating secretary. Thuki provides a premium, secure, and highly isolated generative AI experience directly on your desktop.

## Features

- **Context-Aware**: Intelligently understands your local workflow to provide real-time assistance.
- **Floating Interface**: A sleek, unobtrusive Tauri-based chat interface accessible across your system.
- **Isolated Sandbox**: Generative AI workloads run in a hardened, egress-prohibited Docker container.
- **Privacy-First**: Zero-trust architecture ensures your data never leaves your local environment.

## Architecture & Security

Thuki utilizes a **Dual-Layer Isolation** model for generative inference:

1.  **Frontend (Tauri/React)**: Operates within a secure system webview with restricted IPC.
2.  **Generative Engine (Docker Sandbox)**:
    - **Network Air-Gap**: The engine runs in an internal bridge network with zero internet egress (`internal: true`).
    - **Privilege Dropping**: All Linux kernel capabilities are dropped (`cap_drop: ALL`).
    - **Model Integrity**: Model weights are mounted as Read-Only (`:ro`) to prevent poisoning or persistence by malicious prompts.
    - **Ephemeral State**: All model weights and session data are purged on shutdown using `docker compose down -v`.

## Development

### Prerequisites

- **Bun**: Fast JavaScript runtime and package manager. Install via [bun.sh](https://bun.sh).
- **Rust**: Required for the Tauri backend. Install via [rustup](https://rustup.rs):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
  After installation, restart your shell or run `source ~/.cargo/env` to make `cargo` available.
- **Docker**: Required to run the isolated generative sandbox. Install via [docker.com](https://www.docker.com/get-started).

### Getting Started

1.  **Install Dependencies**:

    ```bash
    bun install
    ```

2.  **Configure Environment** (optional):

    ```bash
    cp .env.example .env
    ```

    Edit `.env` to override defaults. See [docs/configurations.md](docs/configurations.md) for the full reference.

3.  **Start Sandbox (Security-First Launch)**:
    Thuki offers a hardened, isolated Docker sandbox as a secure-by-default environment for generative inference. This is ideal if you do not wish to install AI models directly on your host or prefer maximum isolation from the network.

    This bootstraps the sandbox and pulls the models (default: `llama3.2:3b`).

    ```bash
    bun run sandbox:start
    ```

    _To pre-select or switch the model:_

    ```bash
    OLLAMA_MODEL=llama3:8b bun run sandbox:start
    ```

    > [!TIP]
    > **Skip this step?** If you already have [Ollama](https://ollama.com) installed and running on your local machine (standard port `11434`), Thuki is fully compatible. If your model is already pulled locally, you can skip the sandbox and proceed directly to **Launch Thuki**. Thuki naturally connects to `http://127.0.0.1:11434`.

4.  **Launch Thuki**:
    Starts the Tauri chat interface.

    ```bash
    bun run dev
    ```

    > [!NOTE]
    > **macOS Accessibility Permission**: Thuki registers a global keyboard shortcut to toggle the overlay. This requires macOS Accessibility permission. During development, the system dialog will prompt you to grant permission to your **terminal app** (e.g., iTerm, Terminal) — this is standard macOS behavior for non-bundled binaries and is expected. In production builds (`.app` bundle), the prompt correctly shows "Thuki."

### Production Build

Build a distributable `.app` bundle:

```bash
bun run build:all
```

The bundle is output to `src-tauri/target/release/bundle/macos/Thuki.app`. Launch it directly:

```bash
open src-tauri/target/release/bundle/macos/Thuki.app
```

On first launch, macOS will prompt: **"Thuki would like to control this computer using accessibility features."** Grant it once — this enables the global keyboard shortcut for toggling the overlay. The permission persists across app restarts.

> [!TIP]
> To build a debug `.app` bundle (with DevTools access), run `bun run tauri build -- --debug`. The bundle lands in `src-tauri/target/debug/bundle/macos/Thuki.app`.

### Command Reference

| Command                  | Description                                                             |
| :----------------------- | :---------------------------------------------------------------------- |
| `bun run dev`            | Starts the Tauri application in development mode.                       |
| `bun run sandbox:start`  | Bootstraps the isolated Docker sandbox and pulls the models.            |
| `bun run sandbox:stop`   | **Destructive**: Stops the sandbox and wipes the model volume.          |
| `bun run validate-build` | Multi-stage gate: Lints, formats, typechecks, and builds the full app.  |
| `bun run build:all`      | Compiles both the frontend (Vite) and backend (Rust/Tauri) for release. |

## License

Personal and confidential. Proprietary to Quiet Node.
