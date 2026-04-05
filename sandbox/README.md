# Thuki Sandbox

This directory contains the infrastructure-as-code for running the generative engine in an isolated, secure Docker sandbox.

## Architecture

The sandbox separates model initialization from the inference runtime, keeping concerns clean and the attack surface minimal.

- **Storage:** Generative models are stored in a Docker-managed named volume, isolated from your Mac host filesystem.
- **Initialization (`sandbox-init`):** Bootstraps the volume, polls until the daemon is ready, downloads the model (`OLLAMA_MODEL`), then shuts down gracefully with SIGINT to prevent data corruption.
- **Inference (`sandbox-server`):** The production engine. It attaches the model volume in read-only mode (`:ro`), runs with zero Linux capabilities (`cap_drop: ALL`), blocks privilege escalation, and exposes its API on `127.0.0.1` only. The host application connects locally, and no external process can reach the endpoint.

## Security Controls

| Control | Status | Details |
|---|---|---|
| **Host Isolation** | Active | No bind-mounts; Docker-managed volume only |
| **Data Integrity** | Active | `sandbox_models` volume mounted read-only (`:ro`) on inference server |
| **Ingress Isolation** | Active | API bound to `127.0.0.1:11434`; no external process can reach it |
| **Breakout Mitigation** | Active | `cap_drop: ALL` strips every Linux kernel capability |
| **Privilege Control** | Active | `no-new-privileges: true` blocks setuid/setgid escalation |
| **Read-Only Filesystem** | Active | Container root filesystem is read-only; only `/tmp` is writable |
| **Ephemeral Lifecycle** | Active | `bun run sandbox:stop` runs `down -v`, permanently destroying all model weights |
| **Non-Executable Weights** | Active | GGUF format is math-only; no Python/Pickle code execution risk |

> **Note on network egress:** The sandbox does not use `internal: true` on the Docker network. On macOS, Docker Desktop's networking layer does not support `internal: true` alongside host port binding, so the isolation strategy relies on `127.0.0.1` ingress restriction, `cap_drop: ALL`, and the read-only filesystem instead. Outbound connections from the container are not hard-blocked at the network level.

## Usage

The sandbox is intended for:

- Users who do not want to install Ollama or LLM models directly on their host machine
- Environments where strong process isolation and zero host-filesystem writes are required
- Preventing long-term persistence of model weights on the host disk

> **Optional component:** If you already have [Ollama](https://ollama.com) installed and a model pulled on port `11434`, you can skip the sandbox entirely. Thuki connects to `http://127.0.0.1:11434` regardless of whether the server is native or sandboxed.

**Start the sandbox:**

```bash
bun run sandbox:start
```

The first run pulls the model inside the init container, which may take several minutes depending on your connection. Subsequent starts are instant.

**Stop and wipe the sandbox:**

```bash
bun run sandbox:stop
```

This runs `docker compose down -v`, which destroys the Docker volume and permanently removes all downloaded model weights from disk. Nothing persists after this command.
