//! The recorder test rig: fake platform and sink, the rig over `strudeland-record`'s fake
//! source, and the helpers the unit tests and the transcript share.

use super::*;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::json;
use strudeland_record::Source;
use strudeland_record::testing::{FakeHandle, FakeSource, Limits};
use tempfile::TempDir;

pub(super) const RATE: u32 = 1_000;
pub(super) const WAIT: Duration = Duration::from_secs(5);
/// The host time of the take's first frame in these tests.
pub(super) const T0: u64 = 1_000_000_000;

pub(super) fn device(id: &str, name: &str, channels: u16, is_default: bool) -> DeviceInfo {
    DeviceInfo {
        id: id.to_string(),
        name: name.to_string(),
        channels,
        sample_rate: RATE,
        bluetooth: false,
        is_default,
    }
}

pub(super) fn mic() -> DeviceInfo {
    device("fake:mic", "fake mic", 1, true)
}

pub(super) fn usb() -> DeviceInfo {
    device("fake:usb", "usb interface", 4, false)
}

pub(super) struct FakePlatform {
    pub(super) has_prompt: AtomicBool,
    pub(super) permission: Mutex<Permission>,
    /// The status after a request.
    pub(super) answer: Mutex<Permission>,
    pub(super) requests: AtomicUsize,
    /// When set, a request waits for a message on it.
    pub(super) gate: Mutex<Option<mpsc::Receiver<()>>>,
    pub(super) devices: Mutex<Vec<DeviceInfo>>,
}

impl Platform for Arc<FakePlatform> {
    fn has_permission_prompt(&self) -> bool {
        self.has_prompt.load(Ordering::Acquire)
    }
    fn permission(&self) -> Permission {
        *self.permission.lock().unwrap()
    }

    fn request_permission(&self) {
        self.requests.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = self.gate.lock().unwrap().take() {
            gate.recv().unwrap();
        }
        *self.permission.lock().unwrap() = *self.answer.lock().unwrap();
    }

    fn devices(&self) -> Result<Vec<DeviceInfo>, RecordError> {
        Ok(self.devices.lock().unwrap().clone())
    }
}

#[derive(Default)]
pub(super) struct FakeSink {
    pub(super) states: Mutex<Vec<StateEvent>>,
    pub(super) levels: AtomicUsize,
}

impl Sink for FakeSink {
    fn state(&self, event: StateEvent) {
        self.states.lock().unwrap().push(event);
    }

    fn level(&self, _: LevelEvent) {
        self.levels.fetch_add(1, Ordering::SeqCst);
    }
}

impl FakeSink {
    pub(super) fn states(&self) -> Vec<StateEvent> {
        self.states.lock().unwrap().clone()
    }

    pub(super) fn last(&self) -> Option<StateEvent> {
        self.states.lock().unwrap().last().cloned()
    }

    /// Waits for a state event matching `pred` and returns it.
    pub(super) fn wait(&self, pred: impl Fn(&StateEvent) -> bool) -> StateEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(e) = self.states().into_iter().find(|e| pred(e)) {
                return e;
            }
            assert!(Instant::now() < deadline, "no matching state event");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

pub(super) struct Rig {
    pub(super) tmp: TempDir,
    pub(super) projects: ProjectState,
    pub(super) core: Arc<Core>,
    pub(super) platform: Arc<FakePlatform>,
    pub(super) sink: Arc<FakeSink>,
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
    let platform = Arc::new(FakePlatform {
        has_prompt: AtomicBool::new(true),
        permission: Mutex::new(Permission::Authorized),
        answer: Mutex::new(Permission::Authorized),
        requests: AtomicUsize::new(0),
        gate: Mutex::new(None),
        devices: Mutex::new(vec![mic(), usb()]),
    });
    let sink = Arc::new(FakeSink::default());
    let (handle_tx, handles) = mpsc::channel();
    let arm_failure: Arc<Mutex<Option<RecordError>>> = Arc::default();
    let failure = Arc::clone(&arm_failure);
    let maker: RecorderMaker = Box::new(move || {
        let tx = handle_tx.clone();
        let failure = Arc::clone(&failure);
        let factory = Box::new(move |cfg: &ArmConfig| {
            if let Some(e) = failure.lock().unwrap().take() {
                return Err(e);
            }
            let known = [mic(), usb()];
            let id = cfg.device_id.as_deref().unwrap_or("fake:mic");
            let d = known
                .iter()
                .find(|d| d.id == id)
                .ok_or(RecordError::NoDevice)?;
            let (source, handle) = FakeSource::new(ArmedInfo {
                device_id: d.id.clone(),
                device_name: d.name.clone(),
                channel: cfg.channel,
                channels: d.channels,
                sample_rate: d.sample_rate,
                bluetooth: d.bluetooth,
            });
            tx.send(handle).unwrap();
            Ok(Box::new(source) as Box<dyn Source>)
        });
        Recorder::with_limits(factory, limits.clone())
    });
    let state = RecorderState::new(
        Box::new(Arc::clone(&platform)),
        Arc::clone(&sink) as Arc<dyn Sink>,
        maker,
    );
    Rig {
        tmp,
        projects,
        core: state.0,
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

    pub(super) fn no_new_handle(&self) {
        assert!(self.handles.lock().unwrap().try_recv().is_err());
    }

    pub(super) fn arm(&self) -> FakeHandle {
        arm(&self.core, &self.prefs(), None, None).unwrap();
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

    /// [`Rig::start`] with these start fields.
    pub(super) fn start_with(
        &self,
        handle: &FakeHandle,
        song_stem: Option<&str>,
        fields: StartFields,
    ) -> Result<StartResult, CommandError> {
        std::thread::scope(|s| {
            let started =
                s.spawn(|| start(&self.core, &self.projects, &self.root, song_stem, fields));
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
