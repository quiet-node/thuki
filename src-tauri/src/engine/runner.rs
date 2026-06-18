//! Async runner actor that drives the pure engine state machine.
//!
//! The actor owns the live child process, the in-flight health poll, and the
//! pending chat waiters. Every transition flows through [`state::step`]; the
//! actor only executes the effects the machine requests. The invariants
//! proven by the tests below: at most one engine process is ever alive, a
//! model switch kills the old process and waits for its confirmed exit
//! before spawning the new one, the latest requested target wins, and every
//! chat waiter resolves with the port on `Loaded` or with a typed
//! [`EnsureError`].
//!
//! All timing goes through `tokio::time` so the tests run under paused time.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

use super::process::{poll_until_healthy, EngineChild, EngineProcess, SpawnArgs};
use super::state::{step, Effect, EngineState, Event, Target};
use crate::config::defaults::{
    ENGINE_COMMAND_QUEUE_CAPACITY, ENGINE_HEALTH_DEADLINE_SECS, ENGINE_HEALTH_POLL_INTERVAL_MS,
};

/// Snapshot of the engine lifecycle published through the status watch.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct EngineStatus {
    /// `"stopped"`, `"starting"`, `"loaded"`, `"stopping"`, or `"failed"`.
    pub state: String,
    /// The current/last target's model path; empty when stopped.
    pub model_path: String,
    /// The serving port; only set while loaded.
    pub port: Option<u16>,
    /// The error message; only set while failed.
    pub error: Option<String>,
}

/// Why an [`EngineHandle::ensure_loaded`] call did not produce a port.
#[derive(Debug, PartialEq)]
pub enum EnsureError {
    /// A newer Ensure replaced this request's target before it loaded.
    Superseded,
    /// Spawn or health check failed.
    StartFailed(String),
}

/// Messages from the handle to the actor task.
enum Command {
    Ensure {
        target: Target,
        reply: oneshot::Sender<Result<u16, EnsureError>>,
    },
    Unload {
        reply: oneshot::Sender<()>,
    },
    Touch,
    SetIdleMinutes(u32),
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// RAII marker for an in-flight LLM request against the engine. While at
/// least one guard is alive the idle sweep treats the engine as active, so
/// the idle timer can never kill the sidecar mid-generation (cold
/// ensure, prefill, and body streaming included). Explicit `unload` and
/// `shutdown` are deliberately NOT blocked by guards: a user-driven eviction
/// or app quit always wins over an in-flight request.
pub struct ActivityGuard {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Cloneable handle to the engine runner actor.
#[derive(Clone)]
pub struct EngineHandle {
    cmd_tx: mpsc::Sender<Command>,
    status_rx: watch::Receiver<EngineStatus>,
    in_flight: Arc<AtomicUsize>,
}

impl EngineHandle {
    /// Spawns the actor task. `idle_minutes == 0` disables idle unload.
    pub fn spawn(
        process: Arc<dyn EngineProcess>,
        idle_minutes: u32,
        idle_check_interval: Duration,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(ENGINE_COMMAND_QUEUE_CAPACITY);
        let (status_tx, status_rx) = watch::channel(status_of(&EngineState::Stopped));
        let core = Core {
            process,
            state: EngineState::Stopped,
            child: None,
            health: None,
            pending_port: 0,
            waiters: Vec::new(),
            status_tx,
        };
        let in_flight = Arc::new(AtomicUsize::new(0));
        tokio::spawn(run_actor(
            core,
            cmd_rx,
            Arc::clone(&in_flight),
            idle_minutes,
            idle_check_interval,
        ));
        Self {
            cmd_tx,
            status_rx,
            in_flight,
        }
    }

    /// Marks an LLM request as in flight for the returned guard's lifetime.
    /// Acquire it before `ensure_loaded` and hold it across the whole
    /// streamed response (body read included); dropping it on any exit path
    /// re-arms idle unload.
    pub fn activity_guard(&self) -> ActivityGuard {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        ActivityGuard {
            in_flight: Arc::clone(&self.in_flight),
        }
    }

    /// Resolves with the port once the target is loaded; waits through any
    /// in-flight transitions (kill, exit confirmation, spawn, health check).
    pub async fn ensure_loaded(&self, target: Target) -> Result<u16, EnsureError> {
        let (reply, rx) = oneshot::channel();
        let send = self.cmd_tx.send(Command::Ensure { target, reply }).await;
        if send.is_err() {
            return Err(EnsureError::StartFailed(
                "engine runner is not running".to_string(),
            ));
        }
        rx.await.unwrap_or_else(|_| {
            Err(EnsureError::StartFailed(
                "engine runner stopped before the model loaded".to_string(),
            ))
        })
    }

    /// Stops the engine and releases its memory. Resolved once the process
    /// exit is confirmed.
    pub async fn unload(&self) {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(Command::Unload { reply }).await.is_ok() {
            let _ = rx.await;
        }
    }

    /// Marks chat activity so the idle-unload timer starts over.
    pub fn touch(&self) {
        let _ = self.cmd_tx.try_send(Command::Touch);
    }

    /// Applies a new idle-unload setting without restarting the actor.
    pub async fn set_idle_minutes(&self, minutes: u32) {
        let _ = self.cmd_tx.send(Command::SetIdleMinutes(minutes)).await;
    }

    /// Kills any live child, confirms its exit, and ends the actor task.
    pub async fn shutdown(&self) {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(Command::Shutdown { reply }).await.is_ok() {
            let _ = rx.await;
        }
    }

    /// A watch receiver that observes every lifecycle change.
    pub fn status(&self) -> watch::Receiver<EngineStatus> {
        self.status_rx.clone()
    }

    /// The current lifecycle snapshot: the status watch's latest value.
    /// Backs the `get_engine_status` command so the Settings panel can seed
    /// its residency line on mount instead of assuming "stopped" until the
    /// next transition event.
    pub fn current_status(&self) -> EngineStatus {
        self.status_rx.borrow().clone()
    }
}

/// Pure projection of the machine state into the published status.
fn status_of(state: &EngineState) -> EngineStatus {
    match state {
        EngineState::Stopped => EngineStatus {
            state: "stopped".to_string(),
            model_path: String::new(),
            port: None,
            error: None,
        },
        EngineState::Starting(target) => EngineStatus {
            state: "starting".to_string(),
            model_path: target.model_path.display().to_string(),
            port: None,
            error: None,
        },
        EngineState::Loaded { target, port } => EngineStatus {
            state: "loaded".to_string(),
            model_path: target.model_path.display().to_string(),
            port: Some(*port),
            error: None,
        },
        EngineState::Stopping { next } => EngineStatus {
            state: "stopping".to_string(),
            model_path: next
                .as_ref()
                .map(|target| target.model_path.display().to_string())
                .unwrap_or_default(),
            port: None,
            error: None,
        },
        EngineState::Failed(error) => EngineStatus {
            state: "failed".to_string(),
            model_path: String::new(),
            port: None,
            error: Some(error.clone()),
        },
    }
}

/// The in-flight health poll for the current spawn.
type HealthFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;

/// A pending `ensure_loaded` call: the target it asked for and the channel
/// that resolves it.
type Waiter = (Target, oneshot::Sender<Result<u16, EnsureError>>);

/// Everything the actor mutates while executing transitions.
struct Core {
    process: Arc<dyn EngineProcess>,
    state: EngineState,
    child: Option<Box<dyn EngineChild>>,
    health: Option<HealthFuture>,
    pending_port: u16,
    waiters: Vec<Waiter>,
    status_tx: watch::Sender<EngineStatus>,
}

impl Core {
    /// Feeds one event through the state machine and executes the requested
    /// effects until the machine settles (a kill chains into an exit
    /// confirmation, which can chain into the next spawn).
    async fn dispatch(&mut self, event: Event) {
        let mut pending = Some(event);
        while let Some(ev) = pending.take() {
            let (next, effect) = step(self.state.clone(), ev);
            self.state = next;
            self.status_tx.send_replace(status_of(&self.state));
            self.settle_waiters();
            match effect {
                Effect::None => {}
                Effect::Kill => {
                    self.health = None;
                    self.kill_child().await;
                    pending = Some(Event::ExitConfirmed);
                }
                Effect::Spawn(target) => {
                    if let Err(error) = self.begin_spawn(&target).await {
                        pending = Some(Event::SpawnFailed(error));
                    }
                }
            }
        }
    }

    /// Resolves every pending waiter when the machine reaches a settling
    /// state: `Loaded` resolves matching targets with the port and the rest
    /// as superseded, `Failed` propagates the error, and `Stopped` (reached
    /// through an unload that aborted an in-flight start) supersedes
    /// whatever was still waiting.
    fn settle_waiters(&mut self) {
        match &self.state {
            EngineState::Loaded { target, port } => {
                let (target, port) = (target.clone(), *port);
                for (requested, reply) in self.waiters.drain(..) {
                    let outcome = if requested == target {
                        Ok(port)
                    } else {
                        Err(EnsureError::Superseded)
                    };
                    let _ = reply.send(outcome);
                }
            }
            EngineState::Failed(error) => {
                let error = error.clone();
                for (_, reply) in self.waiters.drain(..) {
                    let _ = reply.send(Err(EnsureError::StartFailed(error.clone())));
                }
            }
            EngineState::Stopped => {
                for (_, reply) in self.waiters.drain(..) {
                    let _ = reply.send(Err(EnsureError::Superseded));
                }
            }
            _ => {}
        }
    }

    /// Grabs a free port, spawns the process, and arms the health poll. The
    /// health result is consumed by the actor loop, racing against an
    /// unexpected child exit.
    async fn begin_spawn(&mut self, target: &Target) -> Result<(), String> {
        let port = self.process.free_port()?;
        let args = SpawnArgs {
            model_path: target.model_path.clone(),
            mmproj_path: target.mmproj_path.clone(),
            num_ctx: target.num_ctx,
            port,
        };
        let child = self.process.spawn(&args).await?;
        self.child = Some(child);
        self.pending_port = port;
        let process = Arc::clone(&self.process);
        self.health = Some(Box::pin(poll_until_healthy(
            move || {
                let process = Arc::clone(&process);
                async move { process.health_probe(port).await }
            },
            Duration::from_secs(ENGINE_HEALTH_DEADLINE_SECS),
            Duration::from_millis(ENGINE_HEALTH_POLL_INTERVAL_MS),
        )));
        Ok(())
    }

    /// Kills the live child, if any, and waits for its exit to land.
    async fn kill_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().await;
        }
    }
}

/// What woke the actor loop.
enum Wake {
    Cmd(Option<Command>),
    Health(Result<(), String>),
    ChildExit,
    Tick,
}

/// The single actor task: owns the [`Core`], serializes every transition,
/// and multiplexes commands, the health poll, child-exit detection, and the
/// idle timer.
async fn run_actor(
    mut core: Core,
    mut cmd_rx: mpsc::Receiver<Command>,
    in_flight: Arc<AtomicUsize>,
    mut idle_minutes: u32,
    idle_check_interval: Duration,
) {
    let mut last_activity = tokio::time::Instant::now();
    let mut ticker = tokio::time::interval(idle_check_interval);
    loop {
        let health_armed = core.health.is_some();
        let child_armed = core.child.is_some();
        let wake = {
            // health_fut and child_fut are taken as mutable borrows of
            // disjoint fields before the select! macro so the borrow checker
            // can see they do not alias core. The `if health_armed` /
            // `if child_armed` guards disable the branch when the option is
            // None, so the `.expect("armed")` inside each async block is
            // unreachable when the branch is inactive and can never fire.
            let health_fut = core.health.as_mut();
            let child_fut = core.child.as_mut();
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => Wake::Cmd(cmd),
                result = async { health_fut.expect("armed").await }, if health_armed => {
                    Wake::Health(result)
                }
                _ = async { child_fut.expect("armed").wait_exit().await }, if child_armed => {
                    Wake::ChildExit
                }
                _ = ticker.tick() => Wake::Tick,
            }
        };
        match wake {
            Wake::Cmd(Some(Command::Ensure { target, reply })) => {
                last_activity = tokio::time::Instant::now();
                core.waiters.push((target.clone(), reply));
                core.dispatch(Event::Ensure(target)).await;
            }
            Wake::Cmd(Some(Command::Unload { reply })) => {
                core.dispatch(Event::Unload).await;
                let _ = reply.send(());
            }
            Wake::Cmd(Some(Command::Touch)) => {
                last_activity = tokio::time::Instant::now();
            }
            Wake::Cmd(Some(Command::SetIdleMinutes(minutes))) => {
                idle_minutes = minutes;
            }
            Wake::Cmd(Some(Command::Shutdown { reply })) => {
                core.health = None;
                core.kill_child().await;
                core.state = EngineState::Stopped;
                core.status_tx.send_replace(status_of(&core.state));
                let _ = reply.send(());
                break;
            }
            // Every handle is gone; tear down like a shutdown. Pending
            // waiters are dropped, which `ensure_loaded` maps to a typed
            // error.
            Wake::Cmd(None) => {
                core.health = None;
                core.kill_child().await;
                core.state = EngineState::Stopped;
                core.status_tx.send_replace(status_of(&core.state));
                break;
            }
            Wake::Health(Ok(())) => {
                core.health = None;
                let port = core.pending_port;
                // Reset the idle clock so a slow model load cannot be
                // idle-killed immediately: the idle window starts from the
                // moment the engine becomes Loaded, not from when Ensure was
                // received.
                last_activity = tokio::time::Instant::now();
                core.dispatch(Event::SpawnedHealthy { port }).await;
            }
            // Health gave up (deadline or hard failure): the process is
            // running but useless, so kill it before reporting the failure.
            Wake::Health(Err(error)) => {
                core.health = None;
                core.kill_child().await;
                core.dispatch(Event::SpawnFailed(error)).await;
            }
            Wake::ChildExit => {
                core.child = None;
                core.health = None;
                core.dispatch(Event::ChildCrashed(
                    "engine process exited unexpectedly".to_string(),
                ))
                .await;
            }
            Wake::Tick => {
                if in_flight.load(Ordering::SeqCst) > 0 {
                    // An LLM request is in flight (cold ensure, prefill, or
                    // body streaming): treat it as continuous activity so
                    // the idle sweep can never kill the engine
                    // mid-generation. The idle window restarts from the
                    // last tick that observed the request.
                    last_activity = tokio::time::Instant::now();
                } else if idle_minutes > 0
                    && matches!(core.state, EngineState::Loaded { .. })
                    && last_activity.elapsed() >= Duration::from_secs(u64::from(idle_minutes) * 60)
                {
                    core.dispatch(Event::IdleExpired).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::defaults::ENGINE_IDLE_CHECK_INTERVAL_SECS;
    use crate::engine::process::{classify_health_status, HealthVerdict};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Mutex;

    // ── Fakes ──────────────────────────────────────────────────────────

    #[derive(Default)]
    struct FakeInner {
        spawns: Vec<SpawnArgs>,
        spawn_errors: VecDeque<String>,
        ports_handed: u16,
        live: usize,
        max_live: usize,
        kills: usize,
        probes_served: usize,
        log: Vec<String>,
        current_exit: Option<Arc<watch::Sender<bool>>>,
    }

    /// Scriptable [`EngineProcess`]: records every spawn, hands out
    /// sequential ports, serves health probes from a channel (a probe with
    /// no queued result blocks, so paused time never runs away), and exposes
    /// crash injection.
    struct FakeProcess {
        inner: Arc<Mutex<FakeInner>>,
        health_tx: mpsc::UnboundedSender<Result<u16, String>>,
        health_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<Result<u16, String>>>,
        /// When set, every probe answers 503 immediately instead of waiting
        /// for a scripted result; paused time then drives the poll loop all
        /// the way to its deadline.
        always_wait: std::sync::atomic::AtomicBool,
    }

    impl FakeProcess {
        fn new() -> Arc<Self> {
            let (health_tx, health_rx) = mpsc::unbounded_channel();
            Arc::new(Self {
                inner: Arc::new(Mutex::new(FakeInner::default())),
                health_tx,
                health_rx: tokio::sync::Mutex::new(health_rx),
                always_wait: std::sync::atomic::AtomicBool::new(false),
            })
        }

        fn push_health(&self, result: Result<u16, String>) {
            self.health_tx.send(result).expect("receiver lives in self");
        }

        fn push_spawn_error(&self, message: &str) {
            self.inner
                .lock()
                .unwrap()
                .spawn_errors
                .push_back(message.to_string());
        }

        /// Makes the live child exit without a kill being issued.
        fn crash_current(&self) {
            let exit = {
                let mut inner = self.inner.lock().unwrap();
                inner.live -= 1;
                inner.log.push("exit".to_string());
                inner.current_exit.take().expect("a child is live")
            };
            let _ = exit.send(true);
        }

        fn snapshot<T>(&self, read: impl Fn(&FakeInner) -> T) -> T {
            read(&self.inner.lock().unwrap())
        }
    }

    struct FakeChild {
        inner: Arc<Mutex<FakeInner>>,
        exit_tx: Arc<watch::Sender<bool>>,
        exit_rx: watch::Receiver<bool>,
    }

    #[async_trait::async_trait]
    impl EngineChild for FakeChild {
        async fn wait_exit(&mut self) {
            let _ = self.exit_rx.wait_for(|exited| *exited).await;
        }

        async fn kill(&mut self) {
            {
                let mut inner = self.inner.lock().unwrap();
                inner.kills += 1;
                inner.log.push("kill".to_string());
                inner.live -= 1;
                inner.log.push("exit".to_string());
            }
            let _ = self.exit_tx.send(true);
        }
    }

    #[async_trait::async_trait]
    impl EngineProcess for FakeProcess {
        async fn spawn(&self, args: &SpawnArgs) -> Result<Box<dyn EngineChild>, String> {
            let mut inner = self.inner.lock().unwrap();
            if let Some(message) = inner.spawn_errors.pop_front() {
                return Err(message);
            }
            inner.spawns.push(args.clone());
            inner.live += 1;
            inner.max_live = inner.max_live.max(inner.live);
            inner
                .log
                .push(format!("spawn {}", args.model_path.display()));
            let (exit_tx, exit_rx) = watch::channel(false);
            let exit_tx = Arc::new(exit_tx);
            inner.current_exit = Some(Arc::clone(&exit_tx));
            Ok(Box::new(FakeChild {
                inner: Arc::clone(&self.inner),
                exit_tx,
                exit_rx,
            }))
        }

        fn free_port(&self) -> Result<u16, String> {
            let mut inner = self.inner.lock().unwrap();
            let port = 40000 + inner.ports_handed;
            inner.ports_handed += 1;
            Ok(port)
        }

        async fn health_probe(&self, _port: u16) -> Result<u16, String> {
            if self.always_wait.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok(503);
            }
            let result = self
                .health_rx
                .lock()
                .await
                .recv()
                .await
                .expect("sender lives in self");
            self.inner.lock().unwrap().probes_served += 1;
            result
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────

    fn target(name: &str) -> Target {
        Target {
            model_path: PathBuf::from(format!("/models/{name}.gguf")),
            mmproj_path: None,
            num_ctx: 4096,
        }
    }

    fn spawn_handle(process: &Arc<FakeProcess>, idle_minutes: u32) -> EngineHandle {
        EngineHandle::spawn(
            Arc::clone(process) as Arc<dyn EngineProcess>,
            idle_minutes,
            Duration::from_secs(ENGINE_IDLE_CHECK_INTERVAL_SECS),
        )
    }

    async fn load(handle: &EngineHandle, process: &Arc<FakeProcess>, name: &str) -> u16 {
        process.push_health(Ok(200));
        handle.ensure_loaded(target(name)).await.expect("loads")
    }

    /// Lets paused time tick forward until the fake reports the condition.
    async fn wait_until(process: &Arc<FakeProcess>, predicate: impl Fn(&FakeInner) -> bool) {
        while !process.snapshot(&predicate) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }

    async fn wait_for_state(rx: &mut watch::Receiver<EngineStatus>, want: &str) {
        while rx.borrow_and_update().state != want {
            rx.changed().await.expect("status channel open");
        }
    }

    /// Yields enough times for the actor to drain its ready work.
    async fn drain_actor() {
        for _ in 0..32 {
            tokio::task::yield_now().await;
        }
    }

    // ── Pure helpers: classification and poll loop ─────────────────────

    #[test]
    fn classify_health_status_maps_statuses() {
        assert_eq!(classify_health_status(200), HealthVerdict::Ready);
        assert_eq!(classify_health_status(503), HealthVerdict::Wait);
        assert_eq!(classify_health_status(500), HealthVerdict::Fail(500));
        assert_eq!(classify_health_status(404), HealthVerdict::Fail(404));
    }

    #[tokio::test(start_paused = true)]
    async fn poll_until_healthy_ready_immediately() {
        let calls = RefCell::new(0);
        let result = poll_until_healthy(
            || {
                *calls.borrow_mut() += 1;
                async { Ok::<u16, String>(200) }
            },
            Duration::from_secs(5),
            Duration::from_millis(250),
        )
        .await;
        assert_eq!(result, Ok(()));
        assert_eq!(*calls.borrow(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn poll_until_healthy_waits_through_503_and_transport_errors() {
        let script = RefCell::new(VecDeque::from([
            Ok(503),
            Err("connection refused".to_string()),
            Ok(200),
        ]));
        let start = tokio::time::Instant::now();
        let result = poll_until_healthy(
            || {
                let next = script.borrow_mut().pop_front().expect("script covers");
                async move { next }
            },
            Duration::from_secs(5),
            Duration::from_millis(250),
        )
        .await;
        assert_eq!(result, Ok(()));
        assert!(script.borrow().is_empty());
        assert_eq!(start.elapsed(), Duration::from_millis(500));
    }

    #[tokio::test(start_paused = true)]
    async fn poll_until_healthy_deadline_exceeded() {
        let result = poll_until_healthy(
            || async { Ok::<u16, String>(503) },
            Duration::from_secs(1),
            Duration::from_millis(250),
        )
        .await;
        assert_eq!(
            result,
            Err("engine did not become healthy before the deadline".to_string())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn poll_until_healthy_fail_status_aborts() {
        let calls = RefCell::new(0);
        let result = poll_until_healthy(
            || {
                *calls.borrow_mut() += 1;
                async { Ok::<u16, String>(500) }
            },
            Duration::from_secs(5),
            Duration::from_millis(250),
        )
        .await;
        assert_eq!(
            result,
            Err("engine health check returned HTTP 500".to_string())
        );
        assert_eq!(*calls.borrow(), 1);
    }

    // ── Runner: load, reuse, switch ────────────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn ensure_spawns_and_reports_loaded() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        let port = load(&handle, &process, "a").await;

        assert_eq!(port, 40000);
        assert_eq!(
            *handle.status().borrow(),
            EngineStatus {
                state: "loaded".to_string(),
                model_path: "/models/a.gguf".to_string(),
                port: Some(40000),
                error: None,
            }
        );
        assert_eq!(
            process.snapshot(|i| i.spawns.clone()),
            vec![SpawnArgs {
                model_path: PathBuf::from("/models/a.gguf"),
                mmproj_path: None,
                num_ctx: 4096,
                port: 40000,
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn current_status_reports_the_latest_snapshot() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        assert_eq!(handle.current_status().state, "stopped");

        let port = load(&handle, &process, "a").await;
        let status = handle.current_status();
        assert_eq!(status.state, "loaded");
        assert_eq!(status.port, Some(port));
        assert_eq!(status.model_path, "/models/a.gguf");
    }

    #[tokio::test(start_paused = true)]
    async fn ensure_waits_for_health() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);
        process.push_health(Ok(503));
        process.push_health(Ok(503));

        let h = handle.clone();
        let waiter = tokio::spawn(async move { h.ensure_loaded(target("a")).await });
        wait_until(&process, |i| i.probes_served == 2).await;

        assert!(!waiter.is_finished());
        assert_eq!(handle.status().borrow().state, "starting");

        process.push_health(Ok(200));
        assert_eq!(waiter.await.unwrap(), Ok(40000));
        assert_eq!(handle.status().borrow().state, "loaded");
    }

    #[tokio::test(start_paused = true)]
    async fn second_ensure_same_target_reuses() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        let first = load(&handle, &process, "a").await;
        let second = handle.ensure_loaded(target("a")).await.expect("reuses");

        assert_eq!(first, second);
        assert_eq!(process.snapshot(|i| i.spawns.len()), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn ensure_new_target_kills_then_spawns_once_exit_confirmed() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        load(&handle, &process, "a").await;
        let port = load(&handle, &process, "b").await;

        assert_eq!(port, 40001);
        assert_eq!(
            process.snapshot(|i| i.log.clone()),
            vec![
                "spawn /models/a.gguf",
                "kill",
                "exit",
                "spawn /models/b.gguf"
            ]
        );
        assert_eq!(process.snapshot(|i| i.max_live), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn rapid_ensures_converge_to_latest() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        let h1 = handle.clone();
        let w1 = tokio::spawn(async move { h1.ensure_loaded(target("a")).await });
        wait_until(&process, |i| i.spawns.len() == 1).await;
        let h2 = handle.clone();
        let w2 = tokio::spawn(async move { h2.ensure_loaded(target("b")).await });
        wait_until(&process, |i| i.spawns.len() == 2).await;
        let h3 = handle.clone();
        let w3 = tokio::spawn(async move { h3.ensure_loaded(target("c")).await });
        wait_until(&process, |i| i.spawns.len() == 3).await;

        process.push_health(Ok(200));

        assert_eq!(w3.await.unwrap(), Ok(40002));
        assert_eq!(w1.await.unwrap(), Err(EnsureError::Superseded));
        assert_eq!(w2.await.unwrap(), Err(EnsureError::Superseded));
        let status = handle.status().borrow().clone();
        assert_eq!(status.state, "loaded");
        assert_eq!(status.model_path, "/models/c.gguf");
        assert_eq!(process.snapshot(|i| i.max_live), 1);
        assert_eq!(process.snapshot(|i| i.kills), 2);
    }

    // ── Runner: unload, waiters, failures ──────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn unload_kills_and_stops() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        load(&handle, &process, "a").await;
        handle.unload().await;

        assert_eq!(handle.status().borrow().state, "stopped");
        assert_eq!(process.snapshot(|i| i.kills), 1);
        assert_eq!(process.snapshot(|i| i.live), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn chat_waiter_mid_transition_resolves_on_loaded() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);
        process.push_health(Ok(503));

        let h1 = handle.clone();
        let w1 = tokio::spawn(async move { h1.ensure_loaded(target("a")).await });
        wait_until(&process, |i| i.probes_served == 1).await;
        let h2 = handle.clone();
        let w2 = tokio::spawn(async move { h2.ensure_loaded(target("a")).await });
        drain_actor().await;
        assert!(!w1.is_finished());
        assert!(!w2.is_finished());

        process.push_health(Ok(200));

        assert_eq!(w1.await.unwrap(), Ok(40000));
        assert_eq!(w2.await.unwrap(), Ok(40000));
        assert_eq!(process.snapshot(|i| i.spawns.len()), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn spawn_failure_reports_failed_and_waiter_gets_error() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);
        process.push_spawn_error("llama-server missing");

        let result = handle.ensure_loaded(target("a")).await;

        assert_eq!(
            result,
            Err(EnsureError::StartFailed("llama-server missing".to_string()))
        );
        let status = handle.status().borrow().clone();
        assert_eq!(status.state, "failed");
        assert_eq!(status.error, Some("llama-server missing".to_string()));
    }

    #[tokio::test(start_paused = true)]
    async fn superseded_waiter_gets_superseded_error() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        let h1 = handle.clone();
        let w1 = tokio::spawn(async move { h1.ensure_loaded(target("a")).await });
        wait_until(&process, |i| i.spawns.len() == 1).await;
        let h2 = handle.clone();
        let w2 = tokio::spawn(async move { h2.ensure_loaded(target("b")).await });
        wait_until(&process, |i| i.spawns.len() == 2).await;

        process.push_health(Ok(200));

        assert_eq!(w2.await.unwrap(), Ok(40001));
        assert_eq!(w1.await.unwrap(), Err(EnsureError::Superseded));
    }

    #[tokio::test(start_paused = true)]
    async fn health_failure_kills_child_and_reports_failed() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);
        process.push_health(Ok(500));

        let result = handle.ensure_loaded(target("a")).await;

        assert_eq!(
            result,
            Err(EnsureError::StartFailed(
                "engine health check returned HTTP 500".to_string()
            ))
        );
        assert_eq!(handle.status().borrow().state, "failed");
        assert_eq!(process.snapshot(|i| i.kills), 1);
        assert_eq!(process.snapshot(|i| i.live), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn health_deadline_failure_reports_failed() {
        let process = FakeProcess::new();
        process
            .always_wait
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let handle = spawn_handle(&process, 0);

        let result = handle.ensure_loaded(target("a")).await;

        assert_eq!(
            result,
            Err(EnsureError::StartFailed(
                "engine did not become healthy before the deadline".to_string()
            ))
        );
        assert_eq!(handle.status().borrow().state, "failed");
        assert_eq!(process.snapshot(|i| i.kills), 1);
        assert_eq!(process.snapshot(|i| i.live), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn transport_error_during_startup_counts_as_wait() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);
        process.push_health(Err("connection refused".to_string()));
        process.push_health(Ok(200));

        let port = handle.ensure_loaded(target("a")).await.expect("loads");

        assert_eq!(port, 40000);
        assert_eq!(handle.status().borrow().state, "loaded");
    }

    #[tokio::test(start_paused = true)]
    async fn unload_mid_start_resolves_waiter_superseded() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        let h1 = handle.clone();
        let w1 = tokio::spawn(async move { h1.ensure_loaded(target("a")).await });
        wait_until(&process, |i| i.spawns.len() == 1).await;

        handle.unload().await;

        assert_eq!(w1.await.unwrap(), Err(EnsureError::Superseded));
        assert_eq!(handle.status().borrow().state, "stopped");
        assert_eq!(process.snapshot(|i| i.kills), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn crash_emits_failed_status() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        load(&handle, &process, "a").await;
        process.crash_current();

        let mut rx = handle.status();
        wait_for_state(&mut rx, "failed").await;
        assert_eq!(
            rx.borrow().error,
            Some("engine process exited unexpectedly".to_string())
        );
        assert_eq!(process.snapshot(|i| i.live), 0);
        assert_eq!(process.snapshot(|i| i.kills), 0);
    }

    // ── Runner: idle unload ────────────────────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn idle_timeout_unloads() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 1);

        load(&handle, &process, "a").await;

        let mut rx = handle.status();
        wait_for_state(&mut rx, "stopped").await;
        assert_eq!(process.snapshot(|i| i.kills), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn touch_activity_defers_idle() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 1);

        load(&handle, &process, "a").await;

        tokio::time::advance(Duration::from_secs(45)).await;
        drain_actor().await;
        assert_eq!(handle.status().borrow().state, "loaded");

        handle.touch();
        drain_actor().await;
        tokio::time::advance(Duration::from_secs(45)).await;
        drain_actor().await;
        // 90 s since load: without the touch the 60 s tick would have
        // unloaded already.
        assert_eq!(handle.status().borrow().state, "loaded");

        let mut rx = handle.status();
        wait_for_state(&mut rx, "stopped").await;
        assert_eq!(process.snapshot(|i| i.kills), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn set_idle_minutes_applies_live() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        load(&handle, &process, "a").await;

        tokio::time::advance(Duration::from_secs(180)).await;
        drain_actor().await;
        assert_eq!(handle.status().borrow().state, "loaded");

        handle.set_idle_minutes(1).await;

        let mut rx = handle.status();
        wait_for_state(&mut rx, "stopped").await;
        assert_eq!(process.snapshot(|i| i.kills), 1);
    }

    /// Regression: a slow model load must not trigger an idle-unload
    /// immediately after becoming Loaded. The idle clock must start at the
    /// moment the health check returns Ok, not at the Ensure call.
    ///
    /// Scenario: idle_minutes = 1, health takes > 60 virtual seconds to
    /// report Ok (scripted as Wait results while paused time advances past
    /// the threshold), then Ready. Engine must stay Loaded and only unload
    /// one idle minute of inactivity after load completes.
    #[tokio::test(start_paused = true)]
    async fn idle_clock_starts_at_loaded_not_at_ensure() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 1);

        // Start loading; push two Wait probes so the health loop blocks.
        process.push_health(Ok(503));
        process.push_health(Ok(503));
        let h = handle.clone();
        let waiter = tokio::spawn(async move { h.ensure_loaded(target("a")).await });

        // Wait for both Wait probes to be consumed (paused time does not
        // advance automatically; the actor is blocked on the health channel).
        wait_until(&process, |i| i.probes_served == 2).await;

        // Advance virtual time past the 60 s idle threshold while the engine
        // is still Starting (health has not yet returned Ok).
        tokio::time::advance(Duration::from_secs(90)).await;
        drain_actor().await;

        // The engine is still Starting; no idle-kill should have fired.
        assert_eq!(handle.status().borrow().state, "starting");

        // Now let the health check succeed: the engine becomes Loaded and the
        // idle clock resets to now (90 virtual seconds in the past is gone).
        process.push_health(Ok(200));
        assert_eq!(waiter.await.unwrap(), Ok(40000));
        assert_eq!(handle.status().borrow().state, "loaded");

        // Advance only 45 s: still within the idle window. Engine must stay loaded.
        tokio::time::advance(Duration::from_secs(45)).await;
        drain_actor().await;
        assert_eq!(handle.status().borrow().state, "loaded");

        // Advance past the full idle minute from the Loaded moment: now it unloads.
        let mut rx = handle.status();
        wait_for_state(&mut rx, "stopped").await;
        assert_eq!(process.snapshot(|i| i.kills), 1);
    }

    /// An in-flight request (activity guard alive) blocks idle unload for
    /// arbitrarily long: a one-minute idle policy must not SIGKILL the
    /// engine mid-generation. Dropping the guard re-arms the sweep.
    #[tokio::test(start_paused = true)]
    async fn activity_guard_blocks_idle_unload_until_dropped() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 1);

        load(&handle, &process, "a").await;
        let guard = handle.activity_guard();

        // Far past the 60 s idle threshold; the guard keeps it loaded.
        tokio::time::advance(Duration::from_secs(300)).await;
        drain_actor().await;
        assert_eq!(handle.status().borrow().state, "loaded");
        assert_eq!(process.snapshot(|i| i.kills), 0);

        drop(guard);
        let mut rx = handle.status();
        wait_for_state(&mut rx, "stopped").await;
        assert_eq!(process.snapshot(|i| i.kills), 1);
    }

    /// Explicit unload and shutdown are user-driven and always win over an
    /// in-flight request: the guard only blocks the idle sweep.
    #[tokio::test(start_paused = true)]
    async fn explicit_unload_and_shutdown_ignore_activity_guard() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 1);

        load(&handle, &process, "a").await;
        let _guard = handle.activity_guard();
        handle.unload().await;
        assert_eq!(handle.status().borrow().state, "stopped");
        assert_eq!(process.snapshot(|i| i.kills), 1);

        load(&handle, &process, "a").await;
        let _guard2 = handle.activity_guard();
        handle.shutdown().await;
        assert_eq!(handle.status().borrow().state, "stopped");
        assert_eq!(process.snapshot(|i| i.kills), 2);
    }

    // ── Runner: shutdown and teardown ──────────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn shutdown_kills_child() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        load(&handle, &process, "a").await;
        handle.shutdown().await;

        assert_eq!(handle.status().borrow().state, "stopped");
        assert_eq!(process.snapshot(|i| i.kills), 1);
        assert_eq!(process.snapshot(|i| i.live), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_fails_pending_waiter() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        let h1 = handle.clone();
        let w1 = tokio::spawn(async move { h1.ensure_loaded(target("a")).await });
        wait_until(&process, |i| i.spawns.len() == 1).await;

        handle.shutdown().await;

        assert_eq!(
            w1.await.unwrap(),
            Err(EnsureError::StartFailed(
                "engine runner stopped before the model loaded".to_string()
            ))
        );
        assert_eq!(process.snapshot(|i| i.kills), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn commands_after_shutdown_error_cleanly() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        handle.shutdown().await;
        drain_actor().await;

        assert_eq!(
            handle.ensure_loaded(target("a")).await,
            Err(EnsureError::StartFailed(
                "engine runner is not running".to_string()
            ))
        );
        handle.unload().await;
        handle.shutdown().await;
        handle.touch();
        handle.set_idle_minutes(5).await;
        assert_eq!(process.snapshot(|i| i.spawns.len()), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_handle_stops_actor_and_kills_child() {
        let process = FakeProcess::new();
        let handle = spawn_handle(&process, 0);

        load(&handle, &process, "a").await;
        let mut rx = handle.status();
        drop(handle);

        wait_for_state(&mut rx, "stopped").await;
        assert_eq!(process.snapshot(|i| i.kills), 1);
        assert_eq!(process.snapshot(|i| i.live), 0);
    }
}
