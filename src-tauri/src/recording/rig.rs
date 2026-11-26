//! The recorder test rig: [`Takes`] over `strudeland_record::testing`'s fakes, the open
//! project state, and the helpers the unit tests and the transcript share.

use super::*;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;
use strudeland_project::Sidecar;
use strudeland_record::testing::{CollectingSink, FakeHandle, FakePlatform, Limits, fake_maker};
use tempfile::TempDir;

pub(super) const RATE: u32 = 1_000;
pub(super) const WAIT: Duration = Duration::from_secs(5);
/// The host time of the take's first frame in these tests.
pub(super) const T0: u64 = 1_000_000_000;

pub(super) fn mic() -> DeviceInfo {
    FakePlatform::mic()
}

pub(super) fn usb() -> DeviceInfo {
    FakePlatform::usb()
}

pub(super) struct Rig {
    pub(super) tmp: TempDir,
    pub(super) projects: ProjectState,
    pub(super) takes: Arc<Takes>,
    pub(super) platform: Arc<FakePlatform>,
    pub(super) sink: Arc<CollectingSink>,
    pub(super) handles: Mutex<mpsc::Receiver<FakeHandle>>,
    /// The recorder's next arm fails with this.
    pub(super) arm_failure: Arc<Mutex<Option<RecordError>>>,
    pub(super) root: String,
}

pub(super) fn rig_with(limits: Limits) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let projects = ProjectState::default();
    let root =
        projects.open_unwatched(Project::create(&tmp.path().join("alpha"), "alpha").unwrap());
    let platform = Arc::new(FakePlatform::new());
    let sink = Arc::new(CollectingSink::default());
    let arm_failure: Arc<Mutex<Option<RecordError>>> = Arc::default();
    let (maker, handles) = fake_maker(limits, Arc::clone(&arm_failure));
    let takes = Takes::new(
        Box::new(Arc::clone(&platform)),
        Arc::clone(&sink) as Arc<dyn Sink>,
        maker,
        takes_config(),
    );
    Rig {
        tmp,
        projects,
        takes,
        platform,
        sink,
        handles: Mutex::new(handles),
        arm_failure,
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
    pub(super) fn prefs(&self) -> PathBuf {
        self.tmp.path().join("data").join(PREFS_FILE)
    }

    pub(super) fn project(&self) -> Arc<Project> {
        project_for(&self.projects, &self.root).unwrap()
    }

    pub(super) fn recordings_dir(&self) -> PathBuf {
        self.project().root().join("recordings")
    }

    /// The state events so far, as the app emits them.
    pub(super) fn states(&self) -> Vec<StateEvent> {
        self.sink
            .states()
            .into_iter()
            .map(StateEvent::from)
            .collect()
    }

    /// The names in `recordings/` but the template's `.gitkeep`, sorted.
    pub(super) fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.recordings_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != ".gitkeep")
            .collect();
        names.sort();
        names
    }

    pub(super) fn next_handle(&self) -> FakeHandle {
        self.handles.lock().unwrap().recv_timeout(WAIT).unwrap()
    }

    pub(super) fn arm(&self) -> FakeHandle {
        self.takes.arm(&self.prefs(), None, None).unwrap();
        self.next_handle()
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
        std::thread::scope(|s| {
            let started = s.spawn(|| {
                self.takes
                    .start(
                        || self.projects.current_for(&self.root),
                        song_stem,
                        fields.into(),
                    )
                    .map(|stem| StartResult { stem })
            });
            while !started.is_finished() {
                if handle.is_recording() {
                    assert!(handle.push(&[0.25; 10], T0));
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            started.join().unwrap()
        })
    }

    /// Delivers frames `from..to` (host time at the nominal rate) and waits for the writer.
    pub(super) fn push(&self, handle: &FakeHandle, from: u64, to: u64) {
        let block = vec![0.5f32; (to - from) as usize];
        assert!(handle.push(&block, T0 + from * 1_000_000));
        assert!(handle.wait_drained(WAIT));
    }

    /// Arms, starts a take as `main-01` and records 1000 frames.
    pub(super) fn take(&self) -> FakeHandle {
        let handle = self.arm();
        assert_eq!(self.start(&handle, Some("main")).unwrap().stem, "main-01");
        self.push(&handle, 10, 1_000);
        handle
    }
}

pub(super) fn sidecar_of(rig: &Rig, stem: &str) -> Sidecar {
    rig.project().read_sidecar(stem).unwrap().unwrap()
}

/// A competing sidecar prevents no-clobber publication on every supported filesystem.
pub(super) fn block_publication(rig: &Rig) -> PathBuf {
    let path = rig.recordings_dir().join("main-01.json");
    std::fs::write(&path, b"existing sidecar").unwrap();
    path
}
