//! The fakes for the take lifecycle's tests and the app's: a platform, a collecting sink, a
//! recorder maker over [`FakeSource`], and [`TakesRig`], the rig both test suites build on.
//! Reached only through [`crate::testing`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use strudeland_project::{AppData, Project, RecordingInput, Sidecar};

use super::{
    Permission, Platform, RecorderMaker, Sink, TakeError, TakeEvent, TakeStart, Takes, TakesConfig,
};
use crate::recorder::Limits;
use crate::source::{FakeHandle, FakeSource};
use crate::{ArmConfig, ArmedInfo, DeviceInfo, Level, RecordError, Recorder, Source};

/// The sample rate of every fake device.
pub const RATE: u32 = 1_000;
/// The host time of a take's first frame in the rigs.
pub const T0: u64 = 1_000_000_000;
/// How long the rigs and [`CollectingSink::wait`] wait.
pub const WAIT: Duration = Duration::from_secs(5);

/// The OS as a test sets it: the permission, the answer to a request, the device list.
pub struct FakePlatform {
    pub has_prompt: AtomicBool,
    pub permission: Mutex<Permission>,
    /// The status after a request.
    pub answer: Mutex<Permission>,
    pub requests: AtomicUsize,
    /// When set, a request waits for a message on it.
    pub gate: Mutex<Option<mpsc::Receiver<()>>>,
    pub devices: Mutex<Vec<DeviceInfo>>,
}

impl FakePlatform {
    /// With a prompt, authorized, listing [`FakePlatform::mic`] and [`FakePlatform::usb`].
    pub fn new() -> FakePlatform {
        FakePlatform {
            has_prompt: AtomicBool::new(true),
            permission: Mutex::new(Permission::Authorized),
            answer: Mutex::new(Permission::Authorized),
            requests: AtomicUsize::new(0),
            gate: Mutex::new(None),
            devices: Mutex::new(vec![FakePlatform::mic(), FakePlatform::usb()]),
        }
    }

    /// A device at the fakes' 1 kHz rate.
    pub fn device(id: &str, name: &str, channels: u16, is_default: bool) -> DeviceInfo {
        DeviceInfo {
            id: id.to_string(),
            name: name.to_string(),
            channels,
            sample_rate: RATE,
            bluetooth: false,
            is_default,
        }
    }

    /// The default input: one channel.
    pub fn mic() -> DeviceInfo {
        FakePlatform::device("fake:mic", "fake mic", 1, true)
    }

    /// Four channels, not the default.
    pub fn usb() -> DeviceInfo {
        FakePlatform::device("fake:usb", "usb interface", 4, false)
    }
}

impl Default for FakePlatform {
    fn default() -> FakePlatform {
        FakePlatform::new()
    }
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

/// Keeps every state event and counts the levels.
#[derive(Default)]
pub struct CollectingSink {
    states: Mutex<Vec<TakeEvent>>,
    levels: AtomicUsize,
}

impl Sink for CollectingSink {
    fn state(&self, event: TakeEvent) {
        self.states.lock().unwrap().push(event);
    }

    fn level(&self, _: Level) {
        self.levels.fetch_add(1, Ordering::SeqCst);
    }
}

impl CollectingSink {
    pub fn states(&self) -> Vec<TakeEvent> {
        self.states.lock().unwrap().clone()
    }

    pub fn last(&self) -> Option<TakeEvent> {
        self.states.lock().unwrap().last().cloned()
    }

    /// Waits (up to 5 s) for a state event matching `pred` and returns it.
    pub fn wait(&self, pred: impl Fn(&TakeEvent) -> bool) -> TakeEvent {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(e) = self.states().into_iter().find(|e| pred(e)) {
                return e;
            }
            assert!(Instant::now() < deadline, "no matching state event");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The number of level events so far.
    pub fn levels(&self) -> usize {
        self.levels.load(Ordering::SeqCst)
    }
}

/// A recorder maker over [`FakeSource`] with these limits: each arm opens a fake stream on
/// [`FakePlatform::mic`] or [`FakePlatform::usb`] (else `NoDevice`) and sends its handle on the
/// receiver. The next arm fails with the error in `arm_failure` when one is set.
pub fn fake_maker(
    limits: Limits,
    arm_failure: Arc<Mutex<Option<RecordError>>>,
) -> (RecorderMaker, Receiver<FakeHandle>) {
    let (handle_tx, handles) = mpsc::channel();
    let maker: RecorderMaker = Box::new(move || {
        let tx = handle_tx.clone();
        let failure = Arc::clone(&arm_failure);
        let factory = Box::new(move |cfg: &ArmConfig| {
            if let Some(e) = failure.lock().unwrap().take() {
                return Err(e);
            }
            let known = [FakePlatform::mic(), FakePlatform::usb()];
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
    (maker, handles)
}

/// [`Takes`] over the fakes and a project, with the helpers both test suites share. The caller
/// owns the temporary directory (this crate has `tempfile` only as a dev-dependency).
pub struct TakesRig {
    /// The root of the caller's temporary directory.
    dir: PathBuf,
    pub project: Arc<Project>,
    pub takes: Arc<Takes>,
    pub platform: Arc<FakePlatform>,
    pub sink: Arc<CollectingSink>,
    pub handles: Mutex<Receiver<FakeHandle>>,
    /// The recorder's next arm fails with this.
    pub arm_failure: Arc<Mutex<Option<RecordError>>>,
}

impl TakesRig {
    /// Takes over `project`, with its app data under `dir/data`.
    pub fn new(dir: &Path, project: Arc<Project>, limits: Limits, config: TakesConfig) -> TakesRig {
        let platform = Arc::new(FakePlatform::new());
        let sink = Arc::new(CollectingSink::default());
        let arm_failure: Arc<Mutex<Option<RecordError>>> = Arc::default();
        let (maker, handles) = fake_maker(limits, Arc::clone(&arm_failure));
        let takes = Takes::new(
            Box::new(Arc::clone(&platform)),
            Arc::clone(&sink) as Arc<dyn Sink>,
            maker,
            config,
        );
        TakesRig {
            dir: dir.to_path_buf(),
            project,
            takes,
            platform,
            sink,
            handles: Mutex::new(handles),
            arm_failure,
        }
    }

    /// The rig's app data dir, `dir/data`.
    pub fn app_data(&self) -> AppData {
        AppData::at(self.dir.join("data"))
    }

    /// The saved input's file, `dir/data/recording.json` (the name predates the app data
    /// documents; the frozen recording transcript calls it).
    pub fn prefs(&self) -> PathBuf {
        RecordingInput::file(&self.app_data())
    }

    pub fn project(&self) -> Arc<Project> {
        Arc::clone(&self.project)
    }

    pub fn recordings_dir(&self) -> PathBuf {
        self.project.root().join("recordings")
    }

    /// The names in `recordings/` but the template's `.gitkeep`, sorted.
    pub fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.recordings_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != ".gitkeep")
            .collect();
        names.sort();
        names
    }

    pub fn next_handle(&self) -> FakeHandle {
        self.handles.lock().unwrap().recv_timeout(WAIT).unwrap()
    }

    pub fn no_new_handle(&self) {
        assert!(self.handles.lock().unwrap().try_recv().is_err());
    }

    pub fn arm(&self) -> FakeHandle {
        self.takes.arm(&self.app_data(), None, None).unwrap();
        self.next_handle()
    }

    /// The rig's project, as the resolver `start` calls.
    pub fn resolve(&self) -> impl FnOnce() -> Result<Arc<Project>, TakeError> + use<> {
        let project = self.project();
        move || Ok(project)
    }

    /// Starts a take in the project `resolve` returns and delivers its first block (10 frames
    /// at [`T0`]) once it runs.
    pub fn start_in<E: From<TakeError> + Send>(
        &self,
        handle: &FakeHandle,
        resolve: impl FnOnce() -> Result<Arc<Project>, E> + Send,
        song_stem: Option<&str>,
        fields: TakeStart,
    ) -> Result<String, E> {
        std::thread::scope(|s| {
            let started = s.spawn(|| self.takes.start(resolve, song_stem, fields));
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
    pub fn push(&self, handle: &FakeHandle, from: u64, to: u64) {
        let block = vec![0.5f32; (to - from) as usize];
        assert!(handle.push(&block, T0 + from * 1_000_000));
        assert!(handle.wait_drained(WAIT));
    }

    /// Arms, starts a take as `main-01` in the rig's project and records 1000 frames.
    pub fn take(&self) -> FakeHandle {
        let handle = self.arm();
        let stem = self.start_in(&handle, self.resolve(), Some("main"), start_fields());
        assert_eq!(stem.unwrap(), "main-01");
        self.push(&handle, 10, 1_000);
        handle
    }
}

/// The start fields of the rigs' takes: 120 bpm in 4/4, backed by `songs/main.js`.
pub fn start_fields() -> TakeStart {
    TakeStart {
        tempo: serde_json::from_value(serde_json::json!({
            "bpm": 120, "meter": "4/4", "cps": 0.5, "beatsPerBar": 4, "tempoOverride": false,
        }))
        .unwrap(),
        backing: serde_json::from_value(serde_json::json!({
            "song": true, "metronome": false, "countIn": true, "songPath": "songs/main.js",
            "startSection": null, "loopSection": null,
        }))
        .unwrap(),
    }
}

pub fn sidecar_of(rig: &TakesRig, stem: &str) -> Sidecar {
    rig.project().read_sidecar(stem).unwrap().unwrap()
}

/// A competing sidecar prevents no-clobber publication on every supported filesystem.
pub fn block_publication(rig: &TakesRig) -> PathBuf {
    let path = rig.recordings_dir().join("main-01.json");
    std::fs::write(&path, b"existing sidecar").unwrap();
    path
}
