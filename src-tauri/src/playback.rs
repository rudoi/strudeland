//! Live playback: the Tauri commands, state and event forwarder over `strudeland-live` (the
//! engine thread and the audio output).
//!
//! The logic lives in plain functions over [`PlaybackState`], so it is tested without an
//! `AppHandle`: the audio device comes through an output factory (tests pass a `FakeOutput`) and
//! the events go out through [`EventSink`] (a `Channel` in the app, a collector in tests). The
//! `#[tauri::command]` wrappers move `start` and `stop` (which open a device or join threads)
//! onto a blocking thread; `schedule` and `seed` (which only lock briefly and queue) are
//! synchronous commands, so they run on the main thread in the order the webview invoked them (an
//! async command could take the session mutex out of order: a batch using a value before the
//! batch defining it, a seed after the haps it should precede). The session mutex is never held
//! across an `.await`. The anchors' clock is the recording API's
//! `host_now_ns` command.
//!
//! One session at a time. Every command but `playback_start` names the session it was issued
//! against, and a command for any other session is `stale`, so a webview that missed a restart
//! cannot drive the new player. A page reload and the app exit stop the session.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

use serde::Serialize;
use strudeland_live::{Batch, CpalOutput, Event, LiveError, Output, OutputError, Player};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::command_error::{CommandError, CommandErrorKind};

/// Where the forwarder delivers the player's events.
pub trait EventSink: Send + 'static {
    fn send(&self, event: Event);
}

impl EventSink for Channel<Event> {
    fn send(&self, event: Event) {
        let _ = Channel::send(self, event);
    }
}

struct Session {
    id: u64,
    player: Player,
    forwarder: JoinHandle<()>,
}

/// The open playback session, if any.
#[derive(Default)]
pub struct PlaybackState {
    session: Mutex<Option<Session>>,
    next_id: AtomicU64,
}

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    pub session: u64,
}

/// A command naming a session that is no longer the open one.
fn stale() -> CommandError {
    CommandError::new(CommandErrorKind::Stale, "the playback session ended")
}

impl From<LiveError> for CommandError {
    fn from(error: LiveError) -> Self {
        let kind = match error {
            LiveError::Output(_) => CommandErrorKind::Output,
            LiveError::Engine(_) => CommandErrorKind::Engine,
        };
        CommandError::new(kind, error.to_string())
    }
}

impl From<OutputError> for CommandError {
    fn from(error: OutputError) -> Self {
        CommandError::from(LiveError::from(error))
    }
}

impl PlaybackState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Takes the session out (under the lock) and stops it (outside it).
    fn end(&self, only: Option<u64>) -> bool {
        let taken = {
            let mut guard = self.lock();
            match (&*guard, only) {
                (Some(s), Some(id)) if s.id != id => None,
                _ => guard.take(),
            }
        };
        match taken {
            Some(session) => {
                stop_session(session);
                true
            }
            None => false,
        }
    }
}

fn stop_session(session: Session) {
    session.player.stop();
    let _ = session.forwarder.join();
}

/// Replaces any session with a new player on the output the factory makes.
pub fn start(
    state: &PlaybackState,
    seed: i32,
    sink: impl EventSink,
    output: impl FnOnce() -> Result<Box<dyn Output>, OutputError>,
) -> Result<StartResult, CommandError> {
    state.end(None);
    let (player, receiver) = Player::start(output()?, seed)?;
    let forwarder = std::thread::Builder::new()
        .name("strudeland-playback-forwarder".into())
        .spawn(move || {
            for event in receiver {
                sink.send(event);
            }
        })
        .map_err(|e| {
            CommandError::new(CommandErrorKind::Engine, format!("forwarder thread: {e}"))
        })?;
    let id = state.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    // A start racing another start: the later store wins and the earlier session is stopped.
    let replaced = state.lock().replace(Session {
        id,
        player,
        forwarder,
    });
    if let Some(old) = replaced {
        stop_session(old);
    }
    Ok(StartResult { session: id })
}

/// Queues a batch on the session and returns at once.
pub fn schedule(state: &PlaybackState, session: u64, batch: Batch) -> Result<(), CommandError> {
    let guard = state.lock();
    let current = match &*guard {
        Some(s) if s.id == session => s,
        _ => return Err(stale()),
    };
    batch
        .validate()
        .map_err(|m| CommandError::new(CommandErrorKind::BadBatch, m))?;
    current.player.schedule(batch);
    Ok(())
}

/// Stops the session and waits for its threads.
pub fn stop(state: &PlaybackState, session: u64) -> Result<(), CommandError> {
    if state.end(Some(session)) {
        Ok(())
    } else {
        Err(stale())
    }
}

pub fn set_seed(state: &PlaybackState, session: u64, seed: i32) -> Result<(), CommandError> {
    match &*state.lock() {
        Some(s) if s.id == session => {
            s.player.set_seed(seed);
            Ok(())
        }
        _ => Err(stale()),
    }
}

/// Switches the live safety limiter of the session (a new session starts with it off).
pub fn set_limiter(state: &PlaybackState, session: u64, on: bool) -> Result<(), CommandError> {
    match &*state.lock() {
        Some(s) if s.id == session => {
            s.player.set_limiter(on);
            Ok(())
        }
        _ => Err(stale()),
    }
}

/// Ends any session: on a page reload (the webview that owned it is gone) and at the app exit.
/// Idempotent.
pub fn end_session(state: &PlaybackState) {
    state.end(None);
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|e| CommandError::new(CommandErrorKind::Engine, format!("task failed: {e}")))?
}

// The blocking commands take the `AppHandle` and look the state up inside the blocking body: a
// `State<'_>` borrow cannot move onto the blocking thread.

#[tauri::command]
pub async fn playback_start(
    app: AppHandle,
    seed: i32,
    channel: Channel<Event>,
) -> Result<StartResult, CommandError> {
    blocking(move || {
        start(&app.state::<PlaybackState>(), seed, channel, || {
            CpalOutput::default_output().map(|o| Box::new(o) as Box<dyn Output>)
        })
    })
    .await
}

// Synchronous on purpose (in-order on the main thread); see the module docs.
#[tauri::command]
pub fn playback_schedule(
    state: State<'_, PlaybackState>,
    session: u64,
    batch: Batch,
) -> Result<(), CommandError> {
    schedule(&state, session, batch)
}

#[tauri::command]
pub async fn playback_stop(app: AppHandle, session: u64) -> Result<(), CommandError> {
    blocking(move || stop(&app.state::<PlaybackState>(), session)).await
}

// Synchronous on purpose (in-order on the main thread); see the module docs.
#[tauri::command]
pub fn playback_seed(
    state: State<'_, PlaybackState>,
    session: u64,
    seed: i32,
) -> Result<(), CommandError> {
    set_seed(&state, session, seed)
}

// Synchronous on purpose (in-order on the main thread); see the module docs.
#[tauri::command]
pub fn playback_limiter(
    state: State<'_, PlaybackState>,
    session: u64,
    on: bool,
) -> Result<(), CommandError> {
    set_limiter(&state, session, on)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use strudeland_live::testing::{FakeOutput, Pace};
    use strudeland_live::{HapEntry, OutputInfo, ValueEntry};

    #[derive(Clone, Default)]
    struct Collect(Arc<Mutex<Vec<Event>>>);

    impl EventSink for Collect {
        fn send(&self, event: Event) {
            self.0.lock().unwrap().push(event);
        }
    }

    impl Collect {
        fn wait(&self, what: &str, pred: impl Fn(&Event) -> bool) {
            let end = Instant::now() + Duration::from_secs(10);
            while Instant::now() < end {
                if self.0.lock().unwrap().iter().any(&pred) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("no {what} in {:?}", self.0.lock().unwrap());
        }
    }

    fn fake() -> Result<Box<dyn Output>, OutputError> {
        let (out, _handle) = FakeOutput::new(
            OutputInfo {
                sample_rate: 48_000,
                buffer_frames: None,
            },
            Pace::RealTime,
        );
        Ok(Box::new(out))
    }

    fn begin(state: &PlaybackState, sink: &Collect) -> u64 {
        start(state, 0, sink.clone(), fake).expect("starts").session
    }

    fn batch(duration: f64) -> Batch {
        Batch {
            values: vec![ValueEntry(0, serde_json::json!({ "s": "sine" }), None)],
            haps: vec![HapEntry(0.0, duration, 0.0, 0, None)],
            cps: 1.0,
        }
    }

    fn kind<T: std::fmt::Debug>(r: Result<T, CommandError>) -> CommandErrorKind {
        r.expect_err("an error").kind
    }

    #[test]
    fn a_second_start_replaces_the_first() {
        let state = PlaybackState::default();
        let first = Collect::default();
        assert_eq!(begin(&state, &first), 1);
        let second = Collect::default();
        assert_eq!(begin(&state, &second), 2);
        assert_eq!(
            kind(schedule(&state, 1, batch(0.1))),
            CommandErrorKind::Stale
        );
        assert_eq!(kind(stop(&state, 1)), CommandErrorKind::Stale);
        assert_eq!(kind(set_seed(&state, 1, 5)), CommandErrorKind::Stale);
        // The first forwarder was joined: its sink is not shared with anything still running.
        assert_eq!(Arc::strong_count(&first.0), 1);
        stop(&state, 2).unwrap();
    }

    #[test]
    fn a_batch_reaches_the_engine() {
        let state = PlaybackState::default();
        let sink = Collect::default();
        let id = begin(&state, &sink);
        schedule(&state, id, batch(0.1)).unwrap();
        sink.wait(
            "stats with the hap",
            |e| matches!(e, Event::Stats(s) if s.batches == 1 && s.haps == 1),
        );
        stop(&state, id).unwrap();
    }

    #[test]
    fn bad_batches_and_stale_sessions_are_refused() {
        let state = PlaybackState::default();
        assert_eq!(
            kind(schedule(&state, 1, batch(0.1))),
            CommandErrorKind::Stale
        );
        let id = begin(&state, &Collect::default());
        assert_eq!(
            kind(schedule(&state, id, batch(-1.0))),
            CommandErrorKind::BadBatch
        );
        assert_eq!(
            kind(schedule(&state, id + 1, batch(0.1))),
            CommandErrorKind::Stale
        );
        set_seed(&state, id, 7).unwrap();
        assert_eq!(kind(set_seed(&state, id + 1, 7)), CommandErrorKind::Stale);
        stop(&state, id).unwrap();
    }

    #[test]
    fn stop_ends_the_session_once() {
        let state = PlaybackState::default();
        let sink = Collect::default();
        let id = begin(&state, &sink);
        stop(&state, id).unwrap();
        assert_eq!(Arc::strong_count(&sink.0), 1);
        assert_eq!(kind(stop(&state, id)), CommandErrorKind::Stale);
        assert_eq!(
            kind(schedule(&state, id, batch(0.1))),
            CommandErrorKind::Stale
        );
    }

    #[test]
    fn end_session_stops_the_session_and_is_idempotent() {
        let state = PlaybackState::default();
        let id = begin(&state, &Collect::default());
        end_session(&state);
        end_session(&state);
        assert_eq!(kind(stop(&state, id)), CommandErrorKind::Stale);
    }

    #[test]
    fn a_failing_output_leaves_no_session_and_no_id_spent() {
        let state = PlaybackState::default();
        let error =
            start(&state, 0, Collect::default(), || Err(OutputError::NoDevice)).expect_err("fails");
        assert_eq!(error.kind, CommandErrorKind::Output);
        assert_eq!(state.next_id.load(Ordering::SeqCst), 0);
        assert!(state.lock().is_none());
        assert_eq!(begin(&state, &Collect::default()), 1);
        end_session(&state);
    }

    #[test]
    fn the_forwarder_delivers_the_anchor() {
        let state = PlaybackState::default();
        let sink = Collect::default();
        let id = begin(&state, &sink);
        sink.wait("anchor", |e| matches!(e, Event::Anchor { .. }));
        stop(&state, id).unwrap();
    }

    #[test]
    fn schedule_and_seed_are_synchronous_commands() {
        // Tauri runs synchronous commands on the main thread in invoke order; async ones race for
        // the session mutex, so a batch could use a value before the batch that defines it.
        let src = include_str!("playback.rs");
        for name in ["playback_schedule", "playback_seed", "playback_limiter"] {
            assert!(
                src.contains(&format!("pub fn {name}(")),
                "{name} is not synchronous"
            );
            assert!(
                !src.contains(&format!("async fn {name}(")),
                "{name} is async"
            );
        }
    }

    #[test]
    fn the_limiter_switch_reaches_the_engine_and_refuses_a_stale_session() {
        let state = PlaybackState::default();
        assert_eq!(kind(set_limiter(&state, 1, true)), CommandErrorKind::Stale);
        let sink = Collect::default();
        let id = begin(&state, &sink);
        assert_eq!(
            kind(set_limiter(&state, id + 1, true)),
            CommandErrorKind::Stale
        );
        set_limiter(&state, id, true).unwrap();
        // a hap far above the ceiling: the limiter reports its gain reduction
        let loud = Batch {
            values: vec![ValueEntry(
                0,
                serde_json::json!({ "s": "sine", "freq": 220.0, "gain": 1.0, "postgain": 4.0 }),
                Some(strudeland_live::OrbitSpec {
                    orbit: 1.0,
                    channels: vec![0, 1],
                }),
            )],
            haps: vec![HapEntry(0.3, 0.3, 0.3, 0, None)],
            cps: 1.0,
        };
        schedule(&state, id, loud).unwrap();
        sink.wait(
            "a levels frame with the limiter working",
            |e| matches!(e, Event::Levels(f) if f.limit_db < -1.0),
        );
        stop(&state, id).unwrap();
    }

    #[test]
    fn batches_and_seeds_reach_the_engine_in_call_order() {
        let state = PlaybackState::default();
        let sink = Collect::default();
        let id = begin(&state, &sink);
        // value 0 defined by the first batch, used by the next ones, a seed in between
        schedule(&state, id, batch(0.1)).unwrap();
        set_seed(&state, id, 3).unwrap();
        for _ in 0..3 {
            let b = Batch {
                values: vec![],
                haps: vec![HapEntry(0.0, 0.1, 0.0, 0, None)],
                cps: 1.0,
            };
            schedule(&state, id, b).unwrap();
        }
        sink.wait(
            "stats with every hap",
            |e| matches!(e, Event::Stats(s) if s.batches == 4 && s.haps == 4),
        );
        stop(&state, id).unwrap();
        let events = sink.0.lock().unwrap();
        let unknown = events
            .iter()
            .any(|e| matches!(e, Event::Error { message } if message.contains("unknown value")));
        assert!(!unknown, "a hap named a value not yet defined");
    }

    #[test]
    fn errors_and_results_serialise_to_the_wire_names() {
        let stale = serde_json::to_string(&stale()).unwrap();
        assert_eq!(
            stale,
            r#"{"kind":"stale","message":"the playback session ended"}"#
        );
        let result = serde_json::to_string(&StartResult { session: 1 }).unwrap();
        assert_eq!(result, r#"{"session":1}"#);
    }
}
