//! Process seam for the built-in engine.
//!
//! [`EngineProcess`] abstracts spawning and health-probing the bundled
//! `llama-server` binary so the runner actor in [`super::runner`] can be
//! driven entirely by fakes in tests. The real implementation,
//! [`TokioEngineProcess`], is a thin wrapper around `tokio::process` and
//! `reqwest`; all the logic around it (health classification, the startup
//! poll loop, command-line construction) lives in pure functions tested
//! directly.

use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::config::defaults::{
    ENGINE_HEALTH_PROBE_TIMEOUT_SECS, ENGINE_STDERR_TAIL_LINES, ENGINE_STDERR_TAIL_LINE_MAX_BYTES,
};

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
    /// The captured tail of the process's stderr, ready to read once
    /// `wait_exit` has resolved (which drains the stream to EOF). The runner
    /// surfaces this as the crash reason so an engine load failure reports the
    /// engine's own message instead of a generic string. Empty when the
    /// process left no stderr.
    fn stderr_tail(&self) -> String;
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
/// `-m <model> [--mmproj <p>] --ctx-size <n> --host 127.0.0.1 --port <p> --no-webui --parallel 1`.
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
    // Single decode slot. Thuki is single-user, so it never needs parallel
    // slots, and the default (n_parallel = 4) actively hurts: the summon-time
    // warm-up prime and the user's first message can land on different KV
    // slots, so the first message re-does the full system-prompt prefill cold
    // instead of reusing the prime's cache (slow first turn, fast after). One
    // slot also gives the conversation the full --ctx-size instead of ctx / 4.
    argv.push("--parallel".into());
    argv.push("1".into());
    argv
}

/// Pure: turns one captured line's raw bytes into a stored tail line. Lossy
/// UTF-8 so invalid bytes from a corrupt stream never panic, with trailing
/// whitespace (e.g. a `\r` from CRLF) trimmed.
fn finalize_stderr_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim_end().to_string()
}

/// Pure: feeds one chunk of stderr bytes through the bounded line accumulator.
/// `current` carries the in-progress line across chunk boundaries; on each
/// `\n` the completed line is finalized and pushed into the tail ring. Bytes
/// past `ENGINE_STDERR_TAIL_LINE_MAX_BYTES` on a single line are dropped, so
/// peak per-line memory stays bounded regardless of how rarely the stream
/// emits a newline (a hard cap on read buffering, not just retained memory).
fn ingest_stderr_chunk(chunk: &[u8], current: &mut Vec<u8>, tail: &mut VecDeque<String>) {
    for &byte in chunk {
        if byte == b'\n' {
            push_stderr_line(
                tail,
                finalize_stderr_line(current),
                ENGINE_STDERR_TAIL_LINES,
            );
            current.clear();
        } else if current.len() < ENGINE_STDERR_TAIL_LINE_MAX_BYTES {
            current.push(byte);
        }
    }
}

/// Pure: appends a captured line to the bounded tail ring, dropping the oldest
/// line once `max_lines` is exceeded so only the trailing window is kept.
fn push_stderr_line(buf: &mut VecDeque<String>, line: String, max_lines: usize) {
    buf.push_back(line);
    while buf.len() > max_lines {
        buf.pop_front();
    }
}

/// Pure: joins the retained tail lines into one newline-separated string.
fn join_stderr_tail(buf: &VecDeque<String>) -> String {
    buf.iter().cloned().collect::<Vec<_>>().join("\n")
}

/// Drains a child's stderr pipe into the bounded tail ring until EOF. Reads in
/// fixed-size chunks (not unbounded lines) and delegates all splitting and
/// bounding to [`ingest_stderr_chunk`], so a stream that never emits a newline
/// cannot force an unbounded allocation. Coverage-off: thin I/O over the tested
/// ingester; a trailing newline-less line (e.g. a process killed mid-line) is
/// flushed after EOF.
#[cfg_attr(coverage_nightly, coverage(off))]
async fn pump_stderr(pipe: tokio::process::ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    use tokio::io::AsyncReadExt;
    let mut reader = tokio::io::BufReader::new(pipe);
    let mut chunk = [0u8; 4096];
    let mut current: Vec<u8> = Vec::new();
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => ingest_stderr_chunk(&chunk[..n], &mut current, &mut tail.lock().unwrap()),
        }
    }
    if !current.is_empty() {
        push_stderr_line(
            &mut tail.lock().unwrap(),
            finalize_stderr_line(&current),
            ENGINE_STDERR_TAIL_LINES,
        );
    }
}

/// A spawned `llama-server` process. `stderr_tail` is the shared bounded ring
/// the reader task fills; the reader handle is joined on exit so the tail is
/// complete before the runner reads it.
struct TokioChild {
    inner: tokio::process::Child,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    reader: Option<tokio::task::JoinHandle<()>>,
}

#[async_trait]
impl EngineChild for TokioChild {
    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn wait_exit(&mut self) {
        let _ = self.inner.wait().await;
        // Join the reader so the stderr tail is fully drained to EOF (which
        // coincides with the pipe closing at process exit) before the runner
        // reads the crash reason.
        if let Some(reader) = self.reader.take() {
            let _ = reader.await;
        }
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn kill(&mut self) {
        let _ = self.inner.start_kill();
        let _ = self.inner.wait().await;
        if let Some(reader) = self.reader.take() {
            let _ = reader.await;
        }
    }

    #[cfg_attr(coverage_nightly, coverage(off))]
    fn stderr_tail(&self) -> String {
        join_stderr_tail(&self.stderr_tail.lock().unwrap())
    }
}

#[async_trait]
impl EngineProcess for TokioEngineProcess {
    #[cfg_attr(coverage_nightly, coverage(off))]
    async fn spawn(&self, args: &SpawnArgs) -> Result<Box<dyn EngineChild>, String> {
        let mut child = tokio::process::Command::new(&self.binary)
            .args(llama_server_args(args))
            .stdout(std::process::Stdio::null())
            // Capture stderr so a load failure (e.g. "unknown model
            // architecture") reaches the user instead of being discarded.
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| e.to_string())?;

        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        // Drain stderr into the bounded tail ring. The task ends when the pipe
        // closes at process exit; `wait_exit`/`kill` join it.
        let reader = child
            .stderr
            .take()
            .map(|pipe| tokio::spawn(pump_stderr(pipe, Arc::clone(&stderr_tail))));

        Ok(Box::new(TokioChild {
            inner: child,
            stderr_tail,
            reader,
        }))
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
                "--parallel",
                "1",
            ]
        );
    }

    #[test]
    fn finalize_stderr_line_is_lossy_and_trims_trailing() {
        assert_eq!(finalize_stderr_line(b"hello"), "hello");
        // Trailing CR (CRLF) and spaces are trimmed.
        assert_eq!(finalize_stderr_line(b"hello\r"), "hello");
        // Invalid UTF-8 never panics; it becomes the replacement char.
        assert_eq!(finalize_stderr_line(&[b'h', b'i', 0xFF]), "hi\u{FFFD}");
    }

    #[test]
    fn ingest_stderr_chunk_splits_on_newlines_and_carries_across_chunks() {
        let mut tail = VecDeque::new();
        let mut current = Vec::new();
        // No newline yet: nothing pushed, line held in `current`.
        ingest_stderr_chunk(b"ab", &mut current, &mut tail);
        assert!(tail.is_empty());
        // Completes "abc", then starts "d".
        ingest_stderr_chunk(b"c\nd", &mut current, &mut tail);
        assert_eq!(tail.iter().cloned().collect::<Vec<_>>(), vec!["abc"]);
        ingest_stderr_chunk(b"\n", &mut current, &mut tail);
        assert_eq!(tail.iter().cloned().collect::<Vec<_>>(), vec!["abc", "d"]);
    }

    #[test]
    fn ingest_stderr_chunk_caps_an_overlong_newlineless_line() {
        let mut tail = VecDeque::new();
        let mut current = Vec::new();
        // A flood longer than the per-line cap, with no newline, must not grow
        // `current` past the cap: peak read buffering is bounded.
        let flood = vec![b'x'; ENGINE_STDERR_TAIL_LINE_MAX_BYTES + 100];
        ingest_stderr_chunk(&flood, &mut current, &mut tail);
        assert_eq!(current.len(), ENGINE_STDERR_TAIL_LINE_MAX_BYTES);
        assert!(tail.is_empty());
        ingest_stderr_chunk(b"\n", &mut current, &mut tail);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].len(), ENGINE_STDERR_TAIL_LINE_MAX_BYTES);
    }

    #[test]
    fn push_stderr_line_keeps_only_the_trailing_window() {
        let mut buf = VecDeque::new();
        push_stderr_line(&mut buf, "a".to_string(), 2);
        push_stderr_line(&mut buf, "b".to_string(), 2);
        push_stderr_line(&mut buf, "c".to_string(), 2);
        assert_eq!(buf.iter().cloned().collect::<Vec<_>>(), vec!["b", "c"]);
    }

    #[test]
    fn join_stderr_tail_newline_joins_in_order() {
        let mut buf = VecDeque::new();
        assert_eq!(join_stderr_tail(&buf), "");
        push_stderr_line(&mut buf, "first".to_string(), 8);
        push_stderr_line(&mut buf, "second".to_string(), 8);
        assert_eq!(join_stderr_tail(&buf), "first\nsecond");
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
                "--parallel",
                "1",
            ]
        );
    }
}
