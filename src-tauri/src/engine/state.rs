//! Pure residency state machine for the built-in engine process.
//!
//! Encodes the engine lifecycle invariants as a side-effect-free transition
//! function: at most one engine process, never two models resident,
//! kill-then-start with a confirmed exit, latest requested target wins.
//! The machine owns no process, no IO, and no clock; the runner actor feeds
//! it [`Event`]s and executes the [`Effect`] each transition requests.

use std::path::PathBuf;

/// A fully resolved engine configuration the runner can spawn.
///
/// Two targets are interchangeable only when every field matches: a `num_ctx`
/// change is a different target and forces a restart, exactly like a model
/// switch, because the context size is fixed at `llama-server` startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// Absolute path to the GGUF model file.
    pub model_path: PathBuf,
    /// Optional multimodal projector file for vision-capable models.
    pub mmproj_path: Option<PathBuf>,
    /// Context window size (tokens) the server is started with.
    pub num_ctx: u32,
}

/// Where the engine process is in its lifecycle.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineState {
    /// No process exists and none is wanted.
    Stopped,
    /// A spawn is in flight for the given target; not yet healthy.
    Starting(Target),
    /// The process is healthy and serving the target on a local port.
    Loaded { target: Target, port: u16 },
    /// A kill was issued and the exit is not yet confirmed. `next` is the
    /// target to spawn once the exit lands, or `None` to stay stopped.
    Stopping { next: Option<Target> },
    /// The last spawn or process died with an error. Sticky until the next
    /// `Ensure` (retry) or `Unload` (acknowledge and return to `Stopped`).
    Failed(String),
}

/// Inputs the runner feeds into the machine: user intent (`Ensure`,
/// `Unload`), process observations (`SpawnedHealthy`, `SpawnFailed`,
/// `ExitConfirmed`, `ChildCrashed`), and timers (`IdleExpired`).
#[derive(Debug, PartialEq)]
pub enum Event {
    /// Make this target resident, restarting the process if a different
    /// target is currently starting or loaded.
    Ensure(Target),
    /// Stop the engine and release its memory. Always wins over a pending
    /// restart queued behind a kill.
    Unload,
    /// The spawned process passed its health check and is serving on `port`.
    SpawnedHealthy { port: u16 },
    /// The spawn attempt itself failed before the process became healthy.
    SpawnFailed(String),
    /// The process exit requested by a kill has been observed.
    ExitConfirmed,
    /// The process died without being asked to.
    ChildCrashed(String),
    /// The idle-unload timer elapsed with no chat activity.
    IdleExpired,
}

/// The single side effect, if any, the runner must execute after a
/// transition. The machine never requests more than one effect per step;
/// chained work (kill then spawn) is sequenced through `Stopping` and a
/// follow-up `ExitConfirmed`.
#[derive(Debug, PartialEq)]
pub enum Effect {
    /// Nothing to do.
    None,
    /// Spawn an engine process for this target.
    Spawn(Target),
    /// Kill the current engine process; an exit confirmation must follow.
    Kill,
}

/// Advances the machine by one event and returns the new state plus the
/// effect the runner must execute.
///
/// Total over all `(state, event)` pairs: combinations outside the lifecycle
/// (late or duplicate process events, redundant user intent) are explicit
/// no-ops, so the runner never needs to pre-filter events.
pub fn step(state: EngineState, event: Event) -> (EngineState, Effect) {
    match (state, event) {
        // Stopped: only Ensure does anything; there is no process to
        // observe, unload, or expire.
        (EngineState::Stopped, Event::Ensure(target)) => {
            (EngineState::Starting(target.clone()), Effect::Spawn(target))
        }
        (state @ EngineState::Stopped, Event::Unload)
        | (state @ EngineState::Stopped, Event::SpawnedHealthy { .. })
        | (state @ EngineState::Stopped, Event::SpawnFailed(_))
        | (state @ EngineState::Stopped, Event::ExitConfirmed)
        | (state @ EngineState::Stopped, Event::ChildCrashed(_))
        | (state @ EngineState::Stopped, Event::IdleExpired) => (state, Effect::None),

        // Starting: waiting on the health check of an in-flight spawn.
        (EngineState::Starting(target), Event::SpawnedHealthy { port }) => {
            (EngineState::Loaded { target, port }, Effect::None)
        }
        (EngineState::Starting(_), Event::SpawnFailed(err))
        | (EngineState::Starting(_), Event::ChildCrashed(err)) => {
            (EngineState::Failed(err), Effect::None)
        }
        (EngineState::Starting(current), Event::Ensure(requested)) => {
            if requested == current {
                // Already starting exactly this target; let the spawn finish.
                (EngineState::Starting(current), Effect::None)
            } else {
                // Different target: abort the in-flight spawn and queue the
                // new target behind the confirmed exit.
                (
                    EngineState::Stopping {
                        next: Some(requested),
                    },
                    Effect::Kill,
                )
            }
        }
        (EngineState::Starting(_), Event::Unload) => {
            (EngineState::Stopping { next: None }, Effect::Kill)
        }
        // A confirmed exit or idle timer cannot belong to a spawn that has
        // not reported healthy yet; ignore.
        (state @ EngineState::Starting(_), Event::ExitConfirmed)
        | (state @ EngineState::Starting(_), Event::IdleExpired) => (state, Effect::None),

        // Loaded: healthy and serving.
        (EngineState::Loaded { target, port }, Event::Ensure(requested)) => {
            if requested == target {
                (EngineState::Loaded { target, port }, Effect::None)
            } else {
                (
                    EngineState::Stopping {
                        next: Some(requested),
                    },
                    Effect::Kill,
                )
            }
        }
        (EngineState::Loaded { .. }, Event::Unload)
        | (EngineState::Loaded { .. }, Event::IdleExpired) => {
            (EngineState::Stopping { next: None }, Effect::Kill)
        }
        (EngineState::Loaded { .. }, Event::ChildCrashed(err)) => {
            (EngineState::Failed(err), Effect::None)
        }
        // A health report or spawn failure cannot apply to an already
        // loaded process; ignore. An ExitConfirmed without a kill is stale.
        (state @ EngineState::Loaded { .. }, Event::SpawnedHealthy { .. })
        | (state @ EngineState::Loaded { .. }, Event::SpawnFailed(_))
        | (state @ EngineState::Loaded { .. }, Event::ExitConfirmed) => (state, Effect::None),

        // Stopping: a kill was issued; everything pivots on the exit.
        // A crash while stopping confirms the exit just as well as the kill
        // landing, so ChildCrashed is handled identically to ExitConfirmed.
        (EngineState::Stopping { next: Some(next) }, Event::ExitConfirmed)
        | (EngineState::Stopping { next: Some(next) }, Event::ChildCrashed(_)) => {
            (EngineState::Starting(next.clone()), Effect::Spawn(next))
        }
        (EngineState::Stopping { next: None }, Event::ExitConfirmed)
        | (EngineState::Stopping { next: None }, Event::ChildCrashed(_)) => {
            (EngineState::Stopped, Effect::None)
        }
        // Latest target wins: replace whatever was queued. The kill is
        // already in flight, so no new effect is needed.
        (EngineState::Stopping { .. }, Event::Ensure(requested)) => (
            EngineState::Stopping {
                next: Some(requested),
            },
            Effect::None,
        ),
        // Manual Unload always wins over a pending restart.
        (EngineState::Stopping { .. }, Event::Unload) => {
            (EngineState::Stopping { next: None }, Effect::None)
        }
        // Health, spawn-failure, and idle events belong to a process
        // generation that has already been superseded; ignore.
        (state @ EngineState::Stopping { .. }, Event::SpawnedHealthy { .. })
        | (state @ EngineState::Stopping { .. }, Event::SpawnFailed(_))
        | (state @ EngineState::Stopping { .. }, Event::IdleExpired) => (state, Effect::None),

        // Failed: sticky error awaiting user intent.
        (EngineState::Failed(_), Event::Ensure(target)) => {
            (EngineState::Starting(target.clone()), Effect::Spawn(target))
        }
        (EngineState::Failed(_), Event::Unload) => (EngineState::Stopped, Effect::None),
        // No process exists in Failed; process and timer events are stale.
        (state @ EngineState::Failed(_), Event::SpawnedHealthy { .. })
        | (state @ EngineState::Failed(_), Event::SpawnFailed(_))
        | (state @ EngineState::Failed(_), Event::ExitConfirmed)
        | (state @ EngineState::Failed(_), Event::ChildCrashed(_))
        | (state @ EngineState::Failed(_), Event::IdleExpired) => (state, Effect::None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target_a() -> Target {
        Target {
            model_path: PathBuf::from("/models/a.gguf"),
            mmproj_path: None,
            num_ctx: 8192,
        }
    }

    fn target_b() -> Target {
        Target {
            model_path: PathBuf::from("/models/b.gguf"),
            mmproj_path: Some(PathBuf::from("/models/b.mmproj.gguf")),
            num_ctx: 8192,
        }
    }

    fn loaded_a() -> EngineState {
        EngineState::Loaded {
            target: target_a(),
            port: 4242,
        }
    }

    // Stopped

    #[test]
    fn ensure_from_stopped_starts() {
        let (state, effect) = step(EngineState::Stopped, Event::Ensure(target_a()));
        assert_eq!(state, EngineState::Starting(target_a()));
        assert_eq!(effect, Effect::Spawn(target_a()));
    }

    #[test]
    fn stopped_ignores_non_ensure_events() {
        for event in [
            Event::Unload,
            Event::SpawnedHealthy { port: 4242 },
            Event::SpawnFailed("boom".into()),
            Event::ExitConfirmed,
            Event::ChildCrashed("boom".into()),
            Event::IdleExpired,
        ] {
            let (state, effect) = step(EngineState::Stopped, event);
            assert_eq!(state, EngineState::Stopped);
            assert_eq!(effect, Effect::None);
        }
    }

    // Starting

    #[test]
    fn health_ok_loads() {
        let (state, effect) = step(
            EngineState::Starting(target_a()),
            Event::SpawnedHealthy { port: 4242 },
        );
        assert_eq!(state, loaded_a());
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn spawn_failed_while_starting_fails() {
        let (state, effect) = step(
            EngineState::Starting(target_a()),
            Event::SpawnFailed("bind error".into()),
        );
        assert_eq!(state, EngineState::Failed("bind error".into()));
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn crash_while_starting_fails() {
        let (state, effect) = step(
            EngineState::Starting(target_a()),
            Event::ChildCrashed("signal 9".into()),
        );
        assert_eq!(state, EngineState::Failed("signal 9".into()));
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn ensure_same_target_while_starting_keeps_starting() {
        let (state, effect) = step(EngineState::Starting(target_a()), Event::Ensure(target_a()));
        assert_eq!(state, EngineState::Starting(target_a()));
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn ensure_while_starting_aborts_inflight() {
        let (state, effect) = step(EngineState::Starting(target_a()), Event::Ensure(target_b()));
        assert_eq!(
            state,
            EngineState::Stopping {
                next: Some(target_b())
            }
        );
        assert_eq!(effect, Effect::Kill);
    }

    #[test]
    fn unload_while_starting_stops() {
        let (state, effect) = step(EngineState::Starting(target_a()), Event::Unload);
        assert_eq!(state, EngineState::Stopping { next: None });
        assert_eq!(effect, Effect::Kill);
    }

    #[test]
    fn starting_ignores_exit_confirmed_and_idle() {
        for event in [Event::ExitConfirmed, Event::IdleExpired] {
            let (state, effect) = step(EngineState::Starting(target_a()), event);
            assert_eq!(state, EngineState::Starting(target_a()));
            assert_eq!(effect, Effect::None);
        }
    }

    // Loaded

    #[test]
    fn ensure_same_target_while_loaded_is_noop() {
        let (state, effect) = step(loaded_a(), Event::Ensure(target_a()));
        assert_eq!(state, loaded_a());
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn ensure_new_target_while_loaded_stops_first() {
        let (state, effect) = step(loaded_a(), Event::Ensure(target_b()));
        assert_eq!(
            state,
            EngineState::Stopping {
                next: Some(target_b())
            }
        );
        assert_eq!(effect, Effect::Kill);
    }

    #[test]
    fn num_ctx_change_is_new_target() {
        let resized = Target {
            num_ctx: 16384,
            ..target_a()
        };
        let (state, effect) = step(loaded_a(), Event::Ensure(resized.clone()));
        assert_eq!(
            state,
            EngineState::Stopping {
                next: Some(resized)
            }
        );
        assert_eq!(effect, Effect::Kill);
    }

    #[test]
    fn unload_while_loaded_stops() {
        let (state, effect) = step(loaded_a(), Event::Unload);
        assert_eq!(state, EngineState::Stopping { next: None });
        assert_eq!(effect, Effect::Kill);
    }

    #[test]
    fn idle_expired_while_loaded_stops() {
        let (state, effect) = step(loaded_a(), Event::IdleExpired);
        assert_eq!(state, EngineState::Stopping { next: None });
        assert_eq!(effect, Effect::Kill);
    }

    #[test]
    fn crash_while_loaded_fails() {
        let (state, effect) = step(loaded_a(), Event::ChildCrashed("oom".into()));
        assert_eq!(state, EngineState::Failed("oom".into()));
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn loaded_ignores_stale_process_events() {
        for event in [
            Event::SpawnedHealthy { port: 9999 },
            Event::SpawnFailed("stale".into()),
            Event::ExitConfirmed,
        ] {
            let (state, effect) = step(loaded_a(), event);
            assert_eq!(state, loaded_a());
            assert_eq!(effect, Effect::None);
        }
    }

    // Stopping

    #[test]
    fn exit_confirmed_with_next_starts_next() {
        let (state, effect) = step(
            EngineState::Stopping {
                next: Some(target_b()),
            },
            Event::ExitConfirmed,
        );
        assert_eq!(state, EngineState::Starting(target_b()));
        assert_eq!(effect, Effect::Spawn(target_b()));
    }

    #[test]
    fn exit_confirmed_without_next_stops() {
        let (state, effect) = step(EngineState::Stopping { next: None }, Event::ExitConfirmed);
        assert_eq!(state, EngineState::Stopped);
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn crash_while_stopping_with_next_starts_next() {
        let (state, effect) = step(
            EngineState::Stopping {
                next: Some(target_b()),
            },
            Event::ChildCrashed("killed".into()),
        );
        assert_eq!(state, EngineState::Starting(target_b()));
        assert_eq!(effect, Effect::Spawn(target_b()));
    }

    #[test]
    fn crash_while_stopping_without_next_stops() {
        let (state, effect) = step(
            EngineState::Stopping { next: None },
            Event::ChildCrashed("killed".into()),
        );
        assert_eq!(state, EngineState::Stopped);
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn latest_target_wins_while_stopping() {
        let (state, effect) = step(
            EngineState::Stopping {
                next: Some(target_a()),
            },
            Event::Ensure(target_b()),
        );
        assert_eq!(
            state,
            EngineState::Stopping {
                next: Some(target_b())
            }
        );
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn unload_while_stopping_clears_next() {
        let (state, effect) = step(
            EngineState::Stopping {
                next: Some(target_a()),
            },
            Event::Unload,
        );
        assert_eq!(state, EngineState::Stopping { next: None });
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn stopping_ignores_superseded_events() {
        for event in [
            Event::SpawnedHealthy { port: 4242 },
            Event::SpawnFailed("stale".into()),
            Event::IdleExpired,
        ] {
            let (state, effect) = step(
                EngineState::Stopping {
                    next: Some(target_a()),
                },
                event,
            );
            assert_eq!(
                state,
                EngineState::Stopping {
                    next: Some(target_a())
                }
            );
            assert_eq!(effect, Effect::None);
        }
    }

    // Failed

    #[test]
    fn ensure_from_failed_retries() {
        let (state, effect) = step(
            EngineState::Failed("boom".into()),
            Event::Ensure(target_a()),
        );
        assert_eq!(state, EngineState::Starting(target_a()));
        assert_eq!(effect, Effect::Spawn(target_a()));
    }

    #[test]
    fn unload_from_failed_stops() {
        let (state, effect) = step(EngineState::Failed("boom".into()), Event::Unload);
        assert_eq!(state, EngineState::Stopped);
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn failed_ignores_stale_process_events() {
        for event in [
            Event::SpawnedHealthy { port: 4242 },
            Event::SpawnFailed("stale".into()),
            Event::ExitConfirmed,
            Event::ChildCrashed("stale".into()),
            Event::IdleExpired,
        ] {
            let (state, effect) = step(EngineState::Failed("boom".into()), event);
            assert_eq!(state, EngineState::Failed("boom".into()));
            assert_eq!(effect, Effect::None);
        }
    }
}
