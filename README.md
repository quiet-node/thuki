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

- **Bun**: Fast JavaScript runtime and package manager.
- **Rust**: Required for the Tauri backend.
- **Docker**: Required to run the isolated generative sandbox.

### Getting Started

1.  **Install Dependencies**:

    ```bash
    bun install
    ```

2.  **Start Sandbox (Security-First Launch)**:
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

3.  **Launch Thuki**:
    Starts the Tauri chat interface.
    ```bash
    bun run dev
    ```

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
