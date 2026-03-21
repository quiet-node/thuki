# Thuki Sandbox

This directory contains the infrastructure-as-code for running the generative engine in a completely isolated, secure Docker sandbox.

## Architecture

To adhere to industry-leading security practices (YAGNI, SOLID, DRY), the architecture separates model initialization from the inference runtime and completely decouples the container from the host system.

- **Storage:** Generative models are stored in a Docker-managed named volume, completely invisible to your Mac host file system.
- **Initialization (`sandbox-init`):** Bootstraps the volume, performs deterministic API readiness checks, downloads the engine dependencies and models (`OLLAMA_MODEL`), and shuts down gracefully with SIGINT to prevent data corruption.
- **Inference (`sandbox-server`):** The production engine. It attaches the model volume in **Read-Only** mode (`:ro`), runs with zero Linux capabilities (`cap_drop: - ALL`), blocks new privileges, and operates inside an egress-prohibited, `internal:true` network constraint. The service is protected by a restart policy to ensure high availability during inference.

## Security Audit Checklist (Zero-Trust Enforcement)

To ensure the highest level of defensibility against generative AI threats, this sandbox implements the following controls:

- [x] **Host Isolation:** zero bind-mounts; container operates in a standard namespace.
- [x] **Data Integrity:** `sandbox_models` attached as **Read-Only (`:ro`)**; prevents poisoning.
- [x] **Network Air-Gap:** `internal: true` disables all internet egress (no "phoning home").
- [x] **Ingress Isolation:** `127.0.0.1` binding ensures ONLY the local host can speak to the API.
- [x] **Breakout Mitigation:** `cap_drop: ALL` strips every Linux kernel capability from the process.
- [x] **Privilege Control:** `no-new-privileges: true` prevents local code execution from gaining root.
- [x] **Resource Limiting:** Hard memory caps (16G) protect the host from resource-exhaustion DoS.
- [x] **Ephemeral Lifecyle:** `down -v` in `package.json` ensures 100% of weights are purged on stop.
- [x] **Non-Executable Format:** Uses GGUF logic-only weights; no Python/Pickle code execution risks.

## Usage

The sandbox is a **security-first** architectural component. It is intended for:

- Users who do not want to install Ollama or LLM models directly on their host machine.
- Environments where high-isolation and zero-egress policies are required.
- Preventing long-term persistence of model weights or generative state on the host disk.

> [!NOTE]
> **Optional Component**: If you already have [Ollama](https://ollama.com) installed and the desired model pulled locally on port `11434`, you can skip this sandbox and use your native instance. Thuki's backend is fully compatible with both native and sandboxed Ollama environments.

**Start the Sandbox:**

```bash
bun run sandbox:start
```

_Note: The first run will pull the model inside the initialization container, which may take several minutes depending on network throughput._

<!- TODO: Make dynamic model selection available for the server ->

```bash
OLLAMA_MODEL=llama3:8b bun run sandbox:start
```

**Stop and Wipe the Sandbox:**

```bash
bun run sandbox:stop
```

_SECURITY FEATURE: This command utilizes the `-v` flag to completely destroy the Docker volume. Every byte of the model weight and any persistent state generated during the session is permanently purged from your host memory and disk._

The isolated sandbox endpoints directly to `http://127.0.0.1:11434`, serving as a drop-in, highly secure replacement for native daemons.
