//! What a reload of the main webview and the app exit stop, and in what order.
//!
//! `lib.rs` only filters the events (the `main` webview's `PageLoadEvent::Started`, which also
//! fires on the first load, when every call is a no-op; and `RunEvent::Exit`) and looks the
//! states up. A recorder or playback state not managed yet is `None` and skipped.
//!
//! [`on_reload`], in this order:
//! 1. [`QuitState::detach`]: a pending Quit request belonged to the old page. An approved
//!    exit keeps its export permit.
//! 2. [`Bridge::detach`]: pending control requests fail with `page reloaded`, and later ones
//!    answer `not ready` until the new page attaches. The socket stays open.
//! 3. [`Takes::on_reload`]: a take in progress is finalized with the fallback sidecar and the
//!    recorder is disarmed, without events.
//! 4. [`playback::end_session`]: the webview that owned the session is gone.
//!
//! Playback stops last here. The order predates this module and is kept: a recorder
//! finalize can delay the stop by the WAV and sidecar write, and a take's song backing plays
//! on superdough, not on this session.
//!
//! [`on_exit`], in this order: playback first, so the audio is the first thing to stop; then
//! the recorder, ahead of the control socket, which could otherwise hold the audio up; then
//! [`Bridge::close_server`] (on Unix it removes the socket, then releases the lock).
//!
//! Left alone on purpose:
//! - A reload does not cancel an audio export. Exports have no cancel mechanism and run to
//!   completion. Until one finishes, a new export answers `busy` and a Quit is refused.
//! - The exit does not reset [`QuitState`] or release the export permit. An approved exit
//!   holds the permit until the process ends, so no export can start between the approval
//!   and the exit.
//! - The project watcher, the piano roll host and the instrument preview. The project stays
//!   open across a reload, a piano roll inspection is bounded, and the preview keeps no
//!   native state. At exit, the end of the process stops them.

#[cfg(test)]
use std::cell::RefCell;

use strudeland_record::takes::Takes;

use crate::app_quit::QuitState;
use crate::control::Bridge;
use crate::playback::{self, PlaybackState};

/// The main webview started a new page load (a reload or HMR).
pub fn on_reload(
    quit: &QuitState,
    bridge: &Bridge,
    takes: Option<&Takes>,
    playback: Option<&PlaybackState>,
) {
    quit.detach();
    step("quit");
    bridge.detach();
    step("bridge");
    if let Some(takes) = takes {
        takes.on_reload();
        step("takes");
    }
    if let Some(state) = playback {
        playback::end_session(state);
        step("playback");
    }
}

/// The app exits (`RunEvent::Exit`).
pub fn on_exit(playback: Option<&PlaybackState>, takes: Option<&Takes>, bridge: &Bridge) {
    // Audio stops first.
    if let Some(state) = playback {
        playback::end_session(state);
        step("playback");
    }
    // Before the control socket, so a slow socket cannot delay the audio.
    if let Some(takes) = takes {
        takes.shutdown();
        step("takes");
    }
    bridge.close_server();
    step("bridge");
}

#[cfg(test)]
thread_local! {
    static STEPS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// Records that a call ran, so the tests can check the order. Does nothing in the app.
#[cfg(test)]
fn step(name: &'static str) {
    STEPS.with(|steps| steps.borrow_mut().push(name));
}

#[cfg(not(test))]
fn step(_: &'static str) {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strudeland_live::testing::{FakeOutput, Pace};
    use strudeland_live::{Event, Output, OutputError, OutputInfo};
    use strudeland_project::Project;
    use strudeland_record::takes::{RecState, TakesConfig};
    use strudeland_record::testing::{Limits, TakesRig};
    use tempfile::TempDir;

    use super::*;
    use crate::command_error::CommandErrorKind;
    use crate::export_audio::ExportState;
    use crate::playback::EventSink;

    struct Discard;

    impl EventSink for Discard {
        fn send(&self, _: Event) {}
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

    /// A playback session on a fake output; returns its id.
    fn session(state: &PlaybackState) -> u64 {
        playback::start(state, 0, Discard, fake)
            .expect("starts")
            .session
    }

    fn assert_stale(state: &PlaybackState, id: u64) {
        let error = playback::stop(state, id).expect_err("the session is gone");
        assert_eq!(error.kind, CommandErrorKind::Stale);
    }

    /// Takes over the project `alpha` in a temporary directory.
    fn rig() -> (TempDir, TakesRig) {
        let tmp = tempfile::tempdir().unwrap();
        let project = Arc::new(Project::create(&tmp.path().join("alpha"), "alpha").unwrap());
        let config = TakesConfig {
            app_version: "test".into(),
        };
        let rig = TakesRig::new(tmp.path(), project, Limits::default(), config);
        (tmp, rig)
    }

    #[test]
    fn a_reload_resets_every_state() {
        let quit = QuitState::default();
        quit.asking();
        let bridge = Bridge::new();
        bridge.attach();
        bridge.serve_idle();
        let (_tmp, rig) = rig();
        let handle = rig.take();
        let state = PlaybackState::default();
        let id = session(&state);

        on_reload(&quit, &bridge, Some(&*rig.takes), Some(&state));

        assert_eq!(quit.view(), (false, None, false));
        // That `Bridge::detach` fails pending requests is pinned by control.rs's
        // `detach_fails_pending_requests_and_later_ones_are_not_ready`.
        assert!(!bridge.attached());
        assert!(bridge.serving(), "the socket stays open across a reload");
        assert!(!handle.is_open());
        assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Idle);
        assert_eq!(rig.files(), ["main-01.json", "main-01.wav"]);
        assert_stale(&state, id);
    }

    #[test]
    fn a_reload_keeps_an_approved_exit() {
        let quit = QuitState::default();
        let exports = ExportState::default();
        quit.approved(&exports);

        on_reload(&quit, &Bridge::new(), None, None);

        assert_eq!(quit.view(), (false, None, true));
        assert!(
            exports.reserve().is_err(),
            "the exit still holds the permit"
        );
    }

    #[test]
    fn the_exit_stops_audio_recorder_and_socket() {
        let state = PlaybackState::default();
        let id = session(&state);
        let (_tmp, rig) = rig();
        let handle = rig.take();
        let bridge = Bridge::new();
        bridge.serve_idle();

        // `on_exit` takes no `QuitState` and no `ExportState`: it cannot reset either.
        on_exit(Some(&state), Some(&*rig.takes), &bridge);

        assert_stale(&state, id);
        assert!(!handle.is_open());
        assert_eq!(rig.takes.status(&rig.app_data()).state, RecState::Idle);
        assert_eq!(rig.files(), ["main-01.json", "main-01.wav"]);
        assert!(!bridge.serving());
    }

    #[test]
    fn the_hooks_skip_states_not_managed_yet() {
        STEPS.take();
        on_reload(&QuitState::default(), &Bridge::new(), None, None);
        assert_eq!(STEPS.take(), ["quit", "bridge"]);
        on_exit(None, None, &Bridge::new());
        assert_eq!(STEPS.take(), ["bridge"]);
    }

    #[test]
    fn the_hooks_run_in_the_documented_order() {
        let quit = QuitState::default();
        let bridge = Bridge::new();
        let (_tmp, rig) = rig();
        let state = PlaybackState::default();

        STEPS.take();
        on_reload(&quit, &bridge, Some(&*rig.takes), Some(&state));
        assert_eq!(STEPS.take(), ["quit", "bridge", "takes", "playback"]);
        on_exit(Some(&state), Some(&*rig.takes), &bridge);
        assert_eq!(STEPS.take(), ["playback", "takes", "bridge"]);
    }
}
