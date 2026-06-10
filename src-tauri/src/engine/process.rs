//! Process seam for the built-in engine.
//!
//! [`EngineProcess`] abstracts spawning and health-probing the bundled
//! `llama-server` binary so the runner actor in [`super::runner`] can be
//! driven entirely by fakes in tests. The real implementation,
//! [`TokioEngineProcess`], is a thin wrapper around `tokio::process` and
//! `reqwest`; all the logic around it (health classification, the startup
//! poll loop, command-line construction) lives in pure functions tested
//! directly.

use std::future::Future;
use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

use crate::config::defaults::ENGINE_HEALTH_PROBE_TIMEOUT_SECS;

/// Everything needed to launch one engine process.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnArgs {
    /// Absolute path to the GGUF model file (`-m`).
    pub model_path: PathBuf,
    /// Optional multimodal projector file for vision models (`--mmproj`).
    pub mmproj_path: Option<PathBuf>,
    /// Context window size in tokens (`--ctx-size`).
    pub num_ctx: u32,
    /// Loopback port the server is told to listen on (`--port`).
    pub port: u16,
}

/// A live engine process the runner can await or kill.
#[async_trait]
pub trait EngineChild: Send {
    /// Resolves when the process exits (normally or by kill).
    async fn wait_exit(&mut self);
    /// Kills the process and waits for the exit to land.
    async fn kill(&mut self);
}

/// Spawn-and-probe seam between the runner actor and the operating system.
#[async_trait]
pub trait EngineProcess: Send + Sync + 'static {
    /// Launches one engine process described by `args`.
    async fn spawn(&self, args: &SpawnArgs) -> Result<Box<dyn EngineChild>, String>;
    /// Binds `127.0.0.1:0` and returns the free port the OS handed out.
    fn free_port(&self) -> Result<u16, String>;
    /// One GET `http://127.0.0.1:{port}/health` returning the raw HTTP
    /// status code (`Err` on transport error). The poll loop and the status
    /// classification are the pure functions in this module; only this
    /// single call is thin.
    async fn health_probe(&self, port: u16) -> Result<u16, String>;
}

/// What one health probe result means for the startup poll loop.
#[derive(Debug, PartialEq)]
pub enum HealthVerdict {
    /// The server is up and the model is loaded.
    Ready,
    /// The server answered but the model is still loading; keep polling.
    Wait,
    /// The server answered with an unexpected status; abort the startup.
    Fail(u16),
}

/// Pure: `200` means ready, `503` means keep waiting (`llama-server` returns
/// 503 while the model loads), anything else is a startup failure.
pub fn classify_health_status(status: u16) -> HealthVerdict {
    match status {
        200 => HealthVerdict::Ready,
        503 => HealthVerdict::Wait,
        other => HealthVerdict::Fail(other),
    }
}

/// Drives `probe` until it reports ready, the deadline is exhausted, or a
/// probe returns a hard failure status. A transport error counts as "keep
/// waiting" because the server socket is not accepting yet during the early
/// part of a spawn.
pub async fn poll_until_healthy<F, Fut>(
    probe: F,
    deadline: Duration,
    interval: Duration,
) -> Result<(), String>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<u16, String>>,
{
    let start = tokio::time::Instant::now();
    loop {
        if start.elapsed() >= deadline {
            return Err("engine did not become healthy before the deadline".to_string());
        }
        if let Ok(status) = probe().await {
            match classify_health_status(status) {
                HealthVerdict::Ready => return Ok(()),
                HealthVerdict::Fail(code) => {
                    return Err(format!("engine health check returned HTTP {code}"));
                }
                HealthVerdict::Wait => {}
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Real [`EngineProcess`] backed by `tokio::process` and `reqwest`.
///
/// Every trait method is a thin OS or network wrapper excluded from
/// coverage; the logic they lean on ([`classify_health_status`],
/// [`poll_until_healthy`], [`llama_server_args`]) is tested directly, and
/// the runner that consumes the trait is tested through fakes.
pub struct TokioEngineProcess {
    /// Configured path to the bundled `llama-server` binary.
    pub binary: PathBuf,
    /// Shared HTTP client used for health probes.
    pub client: reqwest::Client,
}

/// Pure: the `llama-server` command line for one spawn:
/// `-m <model> [--mmproj <p>] --ctx-size <n> --host 127.0.0.1 --port <p> --no-webui`.
fn llama_server_args(args: &SpawnArgs) -> Vec<std::ffi::OsString> {
    let mut argv: Vec<std::ffi::OsString> = vec!["-m".into(), args.model_path.clone().into()];
    if let Some(mmproj) = &args.mmproj_path {
        argv.push("--mmproj".into());
        argv.push(mmproj.clone().into());
    }
    argv.push("--ctx-size".into());
    argv.push(args.num_ctx.to_string().into());
    argv.push("--host".into());
    argv.push("127.0.0.1".into());
    argv.push("--port".into());
    argv.push(args.port.to_string().into());
    argv.push("--no-webui".into());
    argv
}

/// A spawned `llama-server` process.
struct TokioChild {
    inner: tokio::process::Child,
}

#[async_trait]
impl EngineChild for TokioChild {
    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn wait_exit(&mut self) {
        let _ = self.inner.wait().await;
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn kill(&mut self) {
        let _ = self.inner.start_kill();
        let _ = self.inner.wait().await;
    }
}

#[async_trait]
impl EngineProcess for TokioEngineProcess {
    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn spawn(&self, args: &SpawnArgs) -> Result<Box<dyn EngineChild>, String> {
        let child = tokio::process::Command::new(&self.binary)
            .args(llama_server_args(args))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(Box::new(TokioChild { inner: child }))
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn free_port(&self) -> Result<u16, String> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        Ok(listener.local_addr().map_err(|e| e.to_string())?.port())
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn health_probe(&self, port: u16) -> Result<u16, String> {
        let probe = self
            .client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send();
        tokio::time::timeout(Duration::from_secs(ENGINE_HEALTH_PROBE_TIMEOUT_SECS), probe)
            .await
            .map_err(|_| "health probe timed out".to_string())?
            .map(|response| response.status().as_u16())
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(mmproj: Option<&str>) -> SpawnArgs {
        SpawnArgs {
            model_path: PathBuf::from("/models/a.gguf"),
            mmproj_path: mmproj.map(PathBuf::from),
            num_ctx: 8192,
            port: 4242,
        }
    }

    #[test]
    fn llama_server_args_without_mmproj() {
        assert_eq!(
            llama_server_args(&args(None)),
            vec![
                "-m",
                "/models/a.gguf",
                "--ctx-size",
                "8192",
                "--host",
                "127.0.0.1",
                "--port",
                "4242",
                "--no-webui",
            ]
        );
    }

    #[test]
    fn llama_server_args_with_mmproj() {
        assert_eq!(
            llama_server_args(&args(Some("/models/a.mmproj.gguf"))),
            vec![
                "-m",
                "/models/a.gguf",
                "--mmproj",
                "/models/a.mmproj.gguf",
                "--ctx-size",
                "8192",
                "--host",
                "127.0.0.1",
                "--port",
                "4242",
                "--no-webui",
            ]
        );
    }
}
