//! The recorder test rig: `strudeland_record::testing`'s [`TakesRig`] over the open project
//! state, with the wire conversions the unit tests and the transcript share.

use super::*;
use std::ops::Deref;

use serde_json::json;
pub(super) use strudeland_record::testing::{
    FakeHandle, FakePlatform, Limits, RATE, T0, TakesRig, WAIT, sidecar_of,
};
use tempfile::TempDir;

/// [`TakesRig`] over the project `alpha`, opened in `projects` at `root`.
pub(super) struct Rig {
    pub(super) tmp: TempDir,
    pub(super) projects: ProjectState,
    pub(super) core: TakesRig,
    pub(super) root: String,
}

impl Deref for Rig {
    type Target = TakesRig;

    fn deref(&self) -> &TakesRig {
        &self.core
    }
}

pub(super) fn rig_with(limits: Limits) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let projects = ProjectState::default();
    let root =
        projects.open_unwatched(Project::create(&tmp.path().join("alpha"), "alpha").unwrap());
    let project = projects.current_for(&root).unwrap();
    let core = TakesRig::new(tmp.path(), project, limits, takes_config());
    Rig {
        tmp,
        projects,
        core,
        root,
    }
}

pub(super) fn rig() -> Rig {
    rig_with(Limits::default())
}

pub(super) fn start_fields() -> StartFields {
    serde_json::from_value(json!({
        "tempo": { "bpm": 120, "meter": "4/4", "cps": 0.5, "beatsPerBar": 4,
                   "tempoOverride": false },
        "backing": { "song": true, "metronome": false, "countIn": true,
                     "songPath": "songs/main.js", "startSection": null,
                     "loopSection": null },
    }))
    .unwrap()
}

pub(super) fn stop_fields(bar1_host_ns: Option<&str>) -> StopFields {
    serde_json::from_value(json!({
        "output": { "baseLatency": 0.005, "outputLatency": 0.02 },
        "tempo": { "bpm": 90, "meter": "6/8", "cps": 0.5, "beatsPerBar": 3,
                   "tempoOverride": true },
        "backing": { "song": true, "metronome": true, "countIn": false,
                     "songPath": "songs/main.js", "startSection": "verse",
                     "loopSection": null },
        "timing": {
            "bar1HostNs": bar1_host_ns,
            "clockOffsetNs": { "start": { "offsetNs": "-12", "rttNs": "40" }, "end": null },
            "outputStamps": [[0.5, "1000000123"]],
        },
    }))
    .unwrap()
}

impl Rig {
    /// The state events so far, as the app emits them.
    pub(super) fn states(&self) -> Vec<StateEvent> {
        self.sink
            .states()
            .into_iter()
            .map(StateEvent::from)
            .collect()
    }

    /// Starts a take and delivers its first block (10 frames at `T0`) once it runs.
    pub(super) fn start(
        &self,
        handle: &FakeHandle,
        song_stem: Option<&str>,
    ) -> Result<StartResult, CommandError> {
        self.start_with(handle, song_stem, start_fields())
    }

    /// [`Rig::start`] with these start fields, against the open project as `recording_start`.
    pub(super) fn start_with(
        &self,
        handle: &FakeHandle,
        song_stem: Option<&str>,
        fields: StartFields,
    ) -> Result<StartResult, CommandError> {
        self.start_in(
            handle,
            || self.projects.current_for(&self.root),
            song_stem,
            fields.into(),
        )
        .map(|stem| StartResult { stem })
    }
}

/// [`strudeland_record::testing::block_publication`] for this rig, as the transcript's steps
/// take it.
pub(super) fn block_publication(rig: &Rig) -> PathBuf {
    strudeland_record::testing::block_publication(rig)
}
