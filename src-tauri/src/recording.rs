//! Recording: the Tauri commands, state and events over `strudeland-record` (the capture) and
//! `strudeland-project` (the memo files in `recordings/`).
//!
//! The logic lives in plain functions over [`Core`] (the recorder and its [`TakeState`]: idle,
//! armed, recording or ended) and the [`ProjectState`], so it is tested without an `AppHandle`:
//! the microphone permission and the device list come through [`Platform`], the events go out
//! through [`Sink`], and tests build the recorder over `strudeland-record`'s fake source. The
//! `#[tauri::command]` wrappers only resolve paths and move the work onto a blocking thread; the
//! state mutex is only held there, never across an `.await`.
//!
//! A take is finalized into the project captured at start, even if another project was opened
//! meanwhile. Rust picks every name; the webview never writes files and passes no paths.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use strudeland_project::{
    HostNs, Project, ProjectError, RecordingEntry, SIDECAR_SCHEMA, Sidecar, SidecarApp,
    SidecarBacking, SidecarClockOffsets, SidecarInput, SidecarOutput, SidecarTempo, SidecarTiming,
    atomic_write_bytes, format_rfc3339_utc,
};
use strudeland_record::{
    Anchor, ArmConfig, ArmedInfo, DeviceInfo, Ended, Event, RecordError, Recorder, TakeResult,
    frame_at,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::command_error::{CommandError, CommandErrorKind};
use crate::project::ProjectState;

/// `{peak, rms}`, about 30 Hz while armed or recording.
pub const LEVEL_EVENT: &str = "recording://level";
/// `{state, stem?, error?}` on every recorder transition.
pub const STATE_EVENT: &str = "recording://state";
const PREFS_FILE: &str = "recording.json";
const PREFS_SCHEMA: u32 = 1;

const BUSY: &str = "a take is in progress";
const PERMISSION_PENDING: &str = "a microphone access request is pending";
const NOT_ARMED: &str = "the recorder is not armed";
const NO_TAKE: &str = "no take in progress";
const NO_AUDIO: &str = "no audio arrived from the input";
const DENIED: &str = "microphone access denied";
#[cfg(target_os = "macos")]
const RESTRICTED: &str = "microphone access is restricted on this mac";
#[cfg(not(target_os = "macos"))]
const RESTRICTED: &str = "microphone access is restricted by the system";
const LIMIT: &str = "recording stopped at the length limit";

// --- errors ---

impl From<RecordError> for CommandError {
    fn from(e: RecordError) -> Self {
        use CommandErrorKind as K;
        match e {
            RecordError::NoDevice => CommandError::new(K::NoDevice, e.to_string()),
            RecordError::PermissionDenied => CommandError::new(K::PermissionDenied, DENIED),
            // Keep the existing CoreAudio wire classification. On ALSA and WASAPI a busy
            // endpoint is common when another client holds it exclusively.
            #[cfg(target_os = "macos")]
            RecordError::DeviceBusy => CommandError::new(K::DeviceLost, e.to_string()),
            #[cfg(not(target_os = "macos"))]
            RecordError::DeviceBusy => CommandError::busy(e.to_string()),
            RecordError::DeviceLost(_) => CommandError::new(K::DeviceLost, e.to_string()),
            RecordError::AlreadyRecording => CommandError::busy(BUSY),
            RecordError::NotArmed => CommandError::other(NOT_ARMED),
            RecordError::StartTimeout => CommandError::other(NO_AUDIO),
            other => CommandError::other(other.to_string()),
        }
    }
}

// --- wire types ---

/// An input device as `recording_devices` lists it.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub channels: u16,
    pub sample_rate: u32,
    pub is_default: bool,
    pub bluetooth: bool,
}

impl From<&DeviceInfo> for InputDevice {
    fn from(d: &DeviceInfo) -> Self {
        InputDevice {
            id: d.id.clone(),
            name: d.name.clone(),
            channels: d.channels,
            sample_rate: d.sample_rate,
            is_default: d.is_default,
            bluetooth: d.bluetooth,
        }
    }
}

/// The microphone permission (AVFoundation's authorization status).
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "denied and restricted come from AVFoundation")
)]
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    NotDetermined,
    Denied,
    Restricted,
    Authorized,
}

impl Permission {
    /// AVFoundation's raw `AVAuthorizationStatus`; an unknown value is `Denied`.
    #[cfg_attr(
        not(target_os = "macos"),
        allow(dead_code, reason = "called by the AVFoundation status")
    )]
    pub fn from_raw(raw: isize) -> Permission {
        match raw {
            0 => Permission::NotDetermined,
            1 => Permission::Restricted,
            3 => Permission::Authorized,
            _ => Permission::Denied,
        }
    }
}

/// No stream, a stream without a take, or a take.
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecState {
    Idle,
    Armed,
    Recording,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    pub state: RecState,
    pub permission: Permission,
    /// The armed device; `None` in `idle`.
    pub device: Option<InputDevice>,
    /// 0-based: the armed channel, or in `idle` the saved one (0 when none).
    pub channel: u16,
    /// Set only while `recording`.
    pub stem: Option<String>,
    pub elapsed_seconds: Option<f64>,
}

/// A device and its 0-based channel.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputSelection {
    pub device_id: String,
    pub channel: u16,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DevicesResult {
    pub devices: Vec<InputDevice>,
    /// The saved choice resolved against the present devices; `None` without any input device.
    pub selected: Option<InputSelection>,
}

/// What `recording_start` takes: enough for a timing-null sidecar if the webview never stops.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartFields {
    pub tempo: SidecarTempo,
    pub backing: SidecarBacking,
}

/// The webview's part of the sidecar's `timing`.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StopTiming {
    pub bar1_host_ns: Option<HostNs>,
    pub clock_offset_ns: SidecarClockOffsets,
    /// `[contextTime, hostNs]` pairs.
    pub output_stamps: Vec<(f64, HostNs)>,
}

/// What `recording_stop` takes as `sidecar`; Rust adds the rest of the schema.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StopFields {
    pub output: SidecarOutput,
    pub tempo: SidecarTempo,
    pub backing: SidecarBacking,
    pub timing: StopTiming,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartResult {
    pub stem: String,
}

/// `recording://level`: linear amplitude of the chosen channel, 1.0 is full scale.
#[derive(Serialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LevelEvent {
    pub peak: f32,
    pub rms: f32,
}

/// `recording://state`; absent fields are omitted.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StateEvent {
    pub state: RecState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<CommandError>,
}

impl StateEvent {
    fn new(state: RecState) -> StateEvent {
        StateEvent {
            state,
            stem: None,
            error: None,
        }
    }

    fn stem(mut self, stem: impl Into<String>) -> StateEvent {
        self.stem = Some(stem.into());
        self
    }

    fn error(mut self, error: CommandError) -> StateEvent {
        self.error = Some(error);
        self
    }
}

// --- ports ---

/// The OS side: the microphone permission and the input devices.
pub trait Platform: Send + Sync {
    /// The current status; never prompts.
    fn permission(&self) -> Permission;
    /// Whether the OS offers an in-app prompt for an undetermined permission.
    fn has_permission_prompt(&self) -> bool {
        true
    }
    /// Prompts for access and blocks until the user answered.
    fn request_permission(&self);
    fn devices(&self) -> Result<Vec<DeviceInfo>, RecordError>;
}

/// Where the events go: the app emits them, tests collect them.
pub trait Sink: Send + Sync {
    fn state(&self, event: StateEvent);
    fn level(&self, event: LevelEvent);
}

/// Builds the recorder at the first arm.
pub type RecorderMaker = Box<dyn Fn() -> (Recorder, Receiver<Event>) + Send + Sync>;

// --- state ---

/// What a take needs to write its sidecar, from the start on.
struct TakeMeta {
    project: Arc<Project>,
    stem: String,
    start: StartFields,
    input: ArmedInfo,
    created_at: String,
}

struct Take {
    meta: TakeMeta,
    path: PathBuf,
    started: Instant,
}

/// A take Rust finalized by itself (length cap, device loss), kept for the webview's stop.
struct EndedTake {
    meta: TakeMeta,
    result: TakeResult,
    /// False when the fallback publish failed (the partial is still there).
    published: bool,
}

struct ArmedInput {
    /// The recorder's id of this stream, to tell a stale `Failed` from this arm's.
    arm_id: u64,
    device: InputDevice,
    /// What the recorder armed (the take's sidecar `input`).
    info: ArmedInfo,
}

/// Where the recorder's take is. One value, so the combinations nothing reaches (a take
/// without an armed input, a take beside an ended one) cannot be built.
enum TakeState {
    /// No stream. `stream_denied`: the last arm's stream open was refused (desktops without an
    /// in-app prompt report `denied` from it). It is meaningful only here: every way out of idle
    /// is a successful arm, which clears it.
    Idle { stream_denied: bool },
    /// A stream is open on this input, and no take runs.
    Armed(ArmedInput),
    /// A take runs on the armed input.
    Recording { armed: ArmedInput, take: Take },
    /// A take Rust finalized by itself (cap, write error, device loss), or a stop whose publish
    /// failed; kept for the webview's stop. `armed` is `None` once the stream is gone (a device
    /// loss, a `Failed`).
    Ended {
        armed: Option<ArmedInput>,
        ended: EndedTake,
    },
}

impl TakeState {
    /// The armed input, in every state that has one.
    fn armed(&self) -> Option<&ArmedInput> {
        match self {
            TakeState::Armed(armed)
            | TakeState::Recording { armed, .. }
            | TakeState::Ended {
                armed: Some(armed), ..
            } => Some(armed),
            TakeState::Idle { .. } | TakeState::Ended { armed: None, .. } => None,
        }
    }

    fn is_recording(&self) -> bool {
        matches!(self, TakeState::Recording { .. })
    }
}

struct Inner {
    recorder: Option<Recorder>,
    forwarder: Option<JoinHandle<()>>,
    state: TakeState,
    /// The bridge between a `TakeEnded(DeviceLost)` and the `Failed` the recorder sends right
    /// after it: the stem the `Failed`'s idle event carries. Set only by `ended_by_itself` on
    /// `DeviceLost`, taken by every `Failed` (before the stale-id check). It can outlive the
    /// `Ended` state when a stop or disarm runs in between, so it is not part of it.
    pending_loss_stem: Option<String>,
}

impl Default for Inner {
    fn default() -> Self {
        Inner {
            recorder: None,
            forwarder: None,
            state: TakeState::Idle {
                stream_denied: false,
            },
            pending_loss_stem: None,
        }
    }
}

/// The recorder and everything it is doing. One per app (one window).
pub struct Core {
    inner: Mutex<Inner>,
    requesting: AtomicBool,
    platform: Box<dyn Platform>,
    sink: Arc<dyn Sink>,
    maker: RecorderMaker,
}

/// The managed state: one [`Core`], shared with the event forwarder thread.
pub struct RecorderState(pub Arc<Core>);

impl RecorderState {
    pub fn new(platform: Box<dyn Platform>, sink: Arc<dyn Sink>, maker: RecorderMaker) -> Self {
        RecorderState(Arc::new(Core {
            inner: Mutex::new(Inner::default()),
            requesting: AtomicBool::new(false),
            platform,
            sink,
            maker,
        }))
    }
}

impl Core {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock leaves the data consistent, so recover from poison.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, event: StateEvent) {
        self.sink.state(event);
    }
}

/// Creates the recorder and its event forwarder at the first arm.
fn ensure_recorder<'a>(core: &Arc<Core>, inner: &'a mut Inner) -> &'a Recorder {
    if inner.recorder.is_none() {
        let (recorder, events) = (core.maker)();
        let weak = Arc::downgrade(core);
        let forwarder = std::thread::Builder::new()
            .name("recording-events".to_string())
            .spawn(move || forward(weak, events))
            .expect("spawn the recording-events thread");
        inner.recorder = Some(recorder);
        inner.forwarder = Some(forwarder);
    }
    inner
        .recorder
        .as_ref()
        .expect("the recorder was just created")
}

/// Moves to `next`. Leaving an ended take that is not published yet publishes it with the
/// fallback sidecar once `next` is in place; a failure is logged, and the partial stays for the
/// repair at the next open.
fn leave(inner: &mut Inner, next: TakeState) {
    let old = std::mem::replace(&mut inner.state, next);
    if let TakeState::Ended { ended, .. } = old
        && !ended.published
    {
        publish_fallback(&ended.meta, &ended.result);
    }
}

/// Leaves an ended take for its armed input (or idle), publishing it if needed. Any other
/// state is left alone.
fn settle_ended(inner: &mut Inner) {
    if let TakeState::Ended { armed, .. } = &mut inner.state {
        let next = match armed.take() {
            Some(armed) => TakeState::Armed(armed),
            None => TakeState::Idle {
                stream_denied: false,
            },
        };
        leave(inner, next);
    }
}

/// Drains the recorder's events until the channel closes (at shutdown).
fn forward(core: Weak<Core>, events: Receiver<Event>) {
    for event in events {
        let Some(core) = core.upgrade() else {
            return;
        };
        match event {
            Event::Level(level) => core.sink.level(LevelEvent {
                peak: level.peak,
                rms: level.rms,
            }),
            Event::TakeEnded(result) if result.ended != Ended::User => {
                ended_by_itself(&core, result)
            }
            Event::Failed { arm_id, error } => failed(&core, arm_id, error),
            _ => {}
        }
    }
}

/// A take the recorder ended (cap, device loss, write error): publish it with the fallback
/// sidecar and keep it for the webview's stop. A take a command already took is ignored.
fn ended_by_itself(core: &Core, result: TakeResult) {
    let mut inner = core.lock();
    let idle = TakeState::Idle {
        stream_denied: false,
    };
    let (armed, take) = match std::mem::replace(&mut inner.state, idle) {
        TakeState::Recording { armed, take } if take.path == result.path => (armed, take),
        other => {
            inner.state = other;
            return;
        }
    };
    let published = publish_fallback(&take.meta, &result);
    let stem = take.meta.stem.clone();
    let reason = result.ended.clone();
    inner.state = TakeState::Ended {
        armed: Some(armed),
        ended: EndedTake {
            meta: take.meta,
            result,
            published,
        },
    };
    match reason {
        Ended::Cap => core.emit(
            StateEvent::new(RecState::Armed)
                .stem(stem)
                .error(CommandError::new(CommandErrorKind::LimitReached, LIMIT)),
        ),
        // The `Failed` event that follows reports it.
        Ended::DeviceLost(_) => inner.pending_loss_stem = Some(stem),
        Ended::WriteError(message) => core.emit(StateEvent::new(RecState::Armed).stem(stem).error(
            CommandError::other(format!("the take could not be written: {message}")),
        )),
        Ended::User => {}
    }
}

/// The stream of arm `arm_id` died: the recorder is disarmed. A `Failed` of an older arm,
/// queued while the webview armed again, is ignored (the recorder already replaced it).
fn failed(core: &Core, arm_id: u64, error: RecordError) {
    let mut inner = core.lock();
    let stem = inner.pending_loss_stem.take();
    if inner.state.armed().is_some_and(|a| a.arm_id != arm_id) {
        return;
    }
    match &mut inner.state {
        TakeState::Armed(_) => {
            inner.state = TakeState::Idle {
                stream_denied: false,
            }
        }
        TakeState::Ended { armed, .. } => *armed = None,
        TakeState::Idle { .. } => {}
        // Unreachable with a matching id: the recorder sends `TakeEnded` before `Failed`, and
        // `start` sets the take under the lock this waits for. The take is kept and the event
        // is the same: the closest form of dropping the armed input beside a take.
        TakeState::Recording { .. } => {}
    }
    let mut event = StateEvent::new(RecState::Idle).error(error.into());
    event.stem = stem;
    core.emit(event);
}

// --- device prefs ---

/// The saved input choice (`recording.json` in the app data dir). Channel 0-based.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Prefs {
    pub device_id: Option<String>,
    pub channel: u16,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrefsFile {
    schema: u32,
    device_id: Option<String>,
    channel: u16,
}

/// Parses `recording.json`; the error names what is wrong.
pub fn parse_prefs(bytes: &[u8]) -> Result<Prefs, String> {
    let file: PrefsFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if file.schema != PREFS_SCHEMA {
        return Err(format!("schema must be {PREFS_SCHEMA}"));
    }
    Ok(Prefs {
        device_id: file.device_id,
        channel: file.channel,
    })
}

/// The saved prefs, or the defaults when the file is missing or invalid (logged, left in place).
pub fn read_prefs(path: &Path) -> Prefs {
    match std::fs::read(path) {
        Ok(bytes) => parse_prefs(&bytes).unwrap_or_else(|e| {
            eprintln!("recording: ignoring {}: {e}", path.display());
            Prefs::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Prefs::default(),
        Err(e) => {
            eprintln!("recording: cannot read {}: {e}", path.display());
            Prefs::default()
        }
    }
}

pub fn write_prefs(path: &Path, prefs: &Prefs) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let file = PrefsFile {
        schema: PREFS_SCHEMA,
        device_id: prefs.device_id.clone(),
        channel: prefs.channel,
    };
    let mut json = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    json.push('\n');
    atomic_write_bytes(path, json.as_bytes()).map_err(|e| e.to_string())
}

/// The saved device when present, else the default input (else the first); the saved channel
/// when the device has it, else 0. `None` without devices.
pub fn resolve_selection<'a>(
    devices: &'a [DeviceInfo],
    prefs: &Prefs,
) -> Option<(&'a DeviceInfo, u16)> {
    let saved = prefs
        .device_id
        .as_deref()
        .and_then(|id| devices.iter().find(|d| d.id == id));
    let device = saved
        .or_else(|| devices.iter().find(|d| d.is_default))
        .or_else(|| devices.first())?;
    Some((device, saved_channel(device, prefs)))
}

/// The saved channel when `device` has it (or reports no channels: an exclusive endpoint whose
/// config query was refused), else 0.
fn saved_channel(device: &DeviceInfo, prefs: &Prefs) -> u16 {
    if device.channels == 0 || prefs.channel < device.channels {
        prefs.channel
    } else {
        0
    }
}

// --- bar 1 ---

/// The fractional sample index of bar 1's downbeat: `frame_at` over the take's anchors, `None`
/// when there is no bar-1 time, no anchor, or the result falls outside `[0, frames]` (the clocks
/// disagree; logged).
pub fn bar1_frame(
    anchors: &[Anchor],
    sample_rate: u32,
    frames: u64,
    bar1_host_ns: Option<u64>,
) -> Option<f64> {
    let host_ns = bar1_host_ns?;
    let frame = frame_at(anchors, host_ns, sample_rate)?;
    if (0.0..=frames as f64).contains(&frame) {
        Some(frame)
    } else {
        eprintln!("recording: bar 1 at frame {frame} is outside the take's {frames} frames");
        None
    }
}

// --- sidecar ---

/// The sidecar of a take: the webview's `fields`, or with `None` the fallback (start fields,
/// null timing; any part that would fail validation replaced by [`Sidecar::sanitize`], so the
/// fallback always validates).
fn build_sidecar(meta: &TakeMeta, result: &TakeResult, fields: Option<&StopFields>) -> Sidecar {
    let input = &meta.input;
    let mut sidecar = Sidecar {
        schema: SIDECAR_SCHEMA,
        created_at: meta.created_at.clone(),
        app: SidecarApp {
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        input: SidecarInput {
            device_id: input.device_id.clone(),
            device_name: input.device_name.clone(),
            channel: u32::from(input.channel),
            channels: u32::from(input.channels),
            sample_rate: input.sample_rate,
        },
        output: SidecarOutput {
            base_latency: 0.0,
            output_latency: None,
        },
        tempo: meta.start.tempo.clone(),
        backing: meta.start.backing.clone(),
        timing: SidecarTiming {
            bar1_frame: None,
            bar1_host_ns: None,
            clock_offset_ns: SidecarClockOffsets {
                start: None,
                end: None,
            },
            anchors: result
                .anchors
                .iter()
                .map(|a| (a.frame, HostNs(a.host_ns)))
                .collect(),
            output_stamps: Vec::new(),
        },
        xruns: result.xruns,
        recovered: false,
    };
    if let Some(f) = fields {
        let host_ns = f.timing.bar1_host_ns.map(|h| h.0);
        sidecar.output = f.output.clone();
        sidecar.tempo = f.tempo.clone();
        sidecar.backing = f.backing.clone();
        sidecar.timing.bar1_frame =
            bar1_frame(&result.anchors, input.sample_rate, result.frames, host_ns);
        sidecar.timing.bar1_host_ns = f.timing.bar1_host_ns;
        sidecar.timing.clock_offset_ns = f.timing.clock_offset_ns.clone();
        sidecar.timing.output_stamps = f.timing.output_stamps.clone();
    } else if sidecar.sanitize() {
        // The start fields were bad (a `setcps(0)` tempo, say): the audio is kept anyway.
        eprintln!(
            "recording: {} had invalid start fields; the fallback sidecar replaces them",
            meta.stem
        );
    }
    sidecar
}

/// What [`write_sidecar_or_fallback`] does when the webview's fields fail validation.
#[derive(Clone, Copy)]
enum OnInvalid {
    /// Write the fallback sidecar instead.
    WriteFallback,
    /// Keep the sidecar already on disk.
    KeepExisting,
}

/// Writes the take's sidecar (from `fields`, or the fallback with `None`) with `write`. Webview
/// fields that fail validation are logged and handled as `on_invalid` says, so the audio is
/// kept.
fn write_sidecar_or_fallback(
    meta: &TakeMeta,
    result: &TakeResult,
    fields: Option<&StopFields>,
    on_invalid: OnInvalid,
    write: impl Fn(&Sidecar) -> Result<(), ProjectError>,
) -> Result<(), CommandError> {
    match write(&build_sidecar(meta, result, fields)) {
        Err(ProjectError::InvalidSidecar(message)) if fields.is_some() => match on_invalid {
            OnInvalid::WriteFallback => {
                eprintln!("recording: {message}; writing the fallback sidecar");
                write(&build_sidecar(meta, result, None)).map_err(CommandError::for_memos)
            }
            OnInvalid::KeepExisting => {
                eprintln!("recording: {message}; keeping the fallback sidecar");
                Ok(())
            }
        },
        other => other.map_err(CommandError::for_memos),
    }
}

/// The listed entry of `stem`.
fn entry(project: &Project, stem: &str) -> Result<RecordingEntry, CommandError> {
    project
        .recording_entry(stem)
        .map_err(CommandError::for_memos)
}

/// Publishes a finished take's partial as `<stem>.wav` with its sidecar. Webview fields that
/// fail validation fall back to the timing-null sidecar (logged), so the audio is kept. On an
/// error the partial is still there, so the publish can be retried.
fn publish(
    meta: &TakeMeta,
    result: &TakeResult,
    fields: Option<&StopFields>,
) -> Result<(), CommandError> {
    write_sidecar_or_fallback(meta, result, fields, OnInvalid::WriteFallback, |sidecar| {
        meta.project.finish_recording(&meta.stem, sidecar).map(drop)
    })
}

/// Publishes a take Rust ended or stopped with the fallback sidecar, logging a failure; whether
/// it was published.
fn publish_fallback(meta: &TakeMeta, result: &TakeResult) -> bool {
    match publish(meta, result, None) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("recording: could not finalize {}: {}", meta.stem, e.message);
            false
        }
    }
}

/// The webview's stop after Rust finalized the take: replaces the fallback sidecar, or
/// publishes the take when that has not happened yet (marking it published).
fn complete_ended(
    ended: &mut EndedTake,
    fields: Option<&StopFields>,
) -> Result<RecordingEntry, CommandError> {
    let meta = &ended.meta;
    if !ended.published {
        publish(meta, &ended.result, fields)?;
        ended.published = true;
    } else if fields.is_some() {
        write_sidecar_or_fallback(
            meta,
            &ended.result,
            fields,
            OnInvalid::KeepExisting,
            |sidecar| meta.project.write_sidecar(&meta.stem, sidecar),
        )?;
    }
    entry(&meta.project, &meta.stem)
}

// --- commands, as plain functions ---

/// The open project if it is the one at `root`.
fn project_for(projects: &ProjectState, root: &str) -> Result<Arc<Project>, CommandError> {
    projects.current_for(root)
}

pub fn status(core: &Core, prefs_path: &Path) -> RecordingStatus {
    let permission = core.platform.permission();
    let inner = core.lock();
    let armed = inner.state.armed();
    let permission =
        if permission == Permission::NotDetermined && !core.platform.has_permission_prompt() {
            if armed.is_some() {
                Permission::Authorized
            } else if matches!(
                inner.state,
                TakeState::Idle {
                    stream_denied: true
                }
            ) {
                Permission::Denied
            } else {
                permission
            }
        } else {
            permission
        };
    let (state, stem, elapsed_seconds) = match &inner.state {
        TakeState::Recording { take, .. } => (
            RecState::Recording,
            Some(take.meta.stem.clone()),
            Some(take.started.elapsed().as_secs_f64()),
        ),
        _ if armed.is_some() => (RecState::Armed, None, None),
        _ => (RecState::Idle, None, None),
    };
    let (device, channel) = match armed {
        Some(armed) => (Some(armed.device.clone()), armed.info.channel),
        None => (None, read_prefs(prefs_path).channel),
    };
    RecordingStatus {
        state,
        permission,
        device,
        channel,
        stem,
        elapsed_seconds,
    }
}

/// An exclusive input can refuse a second config query while our own stream is open.
fn input_devices(core: &Core) -> Result<Vec<DeviceInfo>, CommandError> {
    let mut list = core.platform.devices()?;
    if let Some(armed) = core.lock().state.armed()
        && let Some(device) = list
            .iter_mut()
            .find(|d| d.id == armed.info.device_id && d.channels == 0)
    {
        device.channels = armed.info.channels;
        device.sample_rate = armed.info.sample_rate;
    }
    Ok(list)
}

pub fn devices(core: &Core, prefs_path: &Path) -> Result<DevicesResult, CommandError> {
    let list = input_devices(core)?;
    let prefs = read_prefs(prefs_path);
    let selected = resolve_selection(&list, &prefs).map(|(d, channel)| InputSelection {
        device_id: d.id.clone(),
        channel,
    });
    Ok(DevicesResult {
        devices: list.iter().map(InputDevice::from).collect(),
        selected,
    })
}

/// Clears `requesting` when the permission request ends, even by a panic.
struct Requesting<'a>(&'a AtomicBool);

impl Drop for Requesting<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The permission, asking first when it is not determined. `Busy` while another arm asks.
fn ensure_permission(core: &Core) -> Result<(), CommandError> {
    let mut permission = core.platform.permission();
    if permission == Permission::NotDetermined {
        if !core.platform.has_permission_prompt() {
            // The actual device open will decide and report access.
            return Ok(());
        }
        if core.requesting.swap(true, Ordering::AcqRel) {
            return Err(CommandError::busy(PERMISSION_PENDING));
        }
        let _requesting = Requesting(&core.requesting);
        core.platform.request_permission();
        // The status, not the handler's answer, decides.
        permission = core.platform.permission();
    }
    let message = match permission {
        Permission::Authorized => return Ok(()),
        Permission::Restricted => RESTRICTED,
        Permission::NotDetermined | Permission::Denied => DENIED,
    };
    let error = CommandError::new(CommandErrorKind::PermissionDenied, message);
    core.emit(StateEvent::new(RecState::Idle).error(error.clone()));
    Err(error)
}

/// Opens the stream on the chosen input (the arguments, else the saved prefs, else the default
/// input). Saves the prefs only when an argument was given.
pub fn arm(
    core: &Arc<Core>,
    prefs_path: &Path,
    device_id: Option<String>,
    channel: Option<u16>,
) -> Result<RecordingStatus, CommandError> {
    if core.requesting.load(Ordering::Acquire) {
        return Err(CommandError::busy(PERMISSION_PENDING));
    }
    if core.lock().state.is_recording() {
        return Err(CommandError::busy(BUSY));
    }
    ensure_permission(core)?;
    let channel_arg = channel;
    let list = input_devices(core)?;
    let prefs = read_prefs(prefs_path);
    let (device, channel) = match device_id.as_deref() {
        Some(id) => {
            let device = list
                .iter()
                .find(|d| d.id == id)
                .ok_or_else(|| CommandError::from(RecordError::NoDevice))?;
            let channel = channel.unwrap_or(saved_channel(device, &prefs));
            (device, channel)
        }
        None => {
            let (device, saved) = resolve_selection(&list, &prefs)
                .ok_or_else(|| CommandError::from(RecordError::NoDevice))?;
            (device, channel.unwrap_or(saved))
        }
    };
    if device.channels != 0 && channel >= device.channels {
        return Err(CommandError::other(format!(
            "channel {channel} is out of range for {} channels",
            device.channels
        )));
    }
    {
        let mut inner = core.lock();
        if inner.state.is_recording() {
            return Err(CommandError::busy(BUSY));
        }
        settle_ended(&mut inner);
        let recorder = ensure_recorder(core, &mut inner);
        let result = recorder.arm_with_id(ArmConfig {
            device_id: Some(device.id.clone()),
            channel,
        });
        let (arm_id, info) = match result {
            Ok(opened) => opened,
            // `settle_ended` ran, so nothing is left to publish: plain assignments.
            Err(RecordError::PermissionDenied) => {
                inner.state = TakeState::Idle {
                    stream_denied: true,
                };
                return Err(RecordError::PermissionDenied.into());
            }
            Err(error) => {
                inner.state = TakeState::Idle {
                    stream_denied: false,
                };
                return Err(error.into());
            }
        };
        inner.state = TakeState::Armed(ArmedInput {
            arm_id,
            device: InputDevice {
                channels: info.channels,
                sample_rate: info.sample_rate,
                ..InputDevice::from(device)
            },
            info,
        });
    }
    if device_id.is_some() || channel_arg.is_some() {
        let prefs = Prefs {
            device_id: Some(device.id.clone()),
            channel,
        };
        if let Err(e) = write_prefs(prefs_path, &prefs) {
            eprintln!("recording: cannot save {}: {e}", prefs_path.display());
        }
    }
    Ok(armed_status(core, prefs_path))
}

fn armed_status(core: &Core, prefs_path: &Path) -> RecordingStatus {
    core.emit(StateEvent::new(RecState::Armed));
    status(core, prefs_path)
}

/// Closes the stream. `Busy` while a take runs.
pub fn disarm(core: &Core, prefs_path: &Path) -> Result<RecordingStatus, CommandError> {
    {
        let mut inner = core.lock();
        if inner.state.is_recording() {
            return Err(CommandError::busy(BUSY));
        }
        settle_ended(&mut inner);
        if let Some(recorder) = &inner.recorder {
            recorder.disarm();
        }
        inner.state = match inner.state {
            TakeState::Idle { stream_denied } => TakeState::Idle { stream_denied },
            _ => TakeState::Idle {
                stream_denied: false,
            },
        };
    }
    core.emit(StateEvent::new(RecState::Idle));
    Ok(status(core, prefs_path))
}

/// Starts a take named by Rust (`<songStem>-NN` or `memo-NN`) in the project at `root`; returns
/// once the writer has received samples.
pub fn start(
    core: &Core,
    projects: &ProjectState,
    root: &str,
    song_stem: Option<&str>,
    fields: StartFields,
) -> Result<StartResult, CommandError> {
    let mut inner = core.lock();
    if inner.state.is_recording() {
        return Err(CommandError::busy(BUSY));
    }
    let (Some(armed), Some(recorder)) = (inner.state.armed(), &inner.recorder) else {
        return Err(CommandError::other(NOT_ARMED));
    };
    let project = project_for(projects, root)?;
    let stem = project
        .default_recording_stem(song_stem)
        .map_err(CommandError::for_memos)?;
    let path = project
        .recording_partial_path(&stem)
        .map_err(CommandError::for_memos)?;
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    recorder.start(&path)?;
    let input = armed.info.clone();
    // Move the armed input out. An ended take stays in the state, so `leave` publishes it if it
    // is not published yet.
    let armed = match &mut inner.state {
        TakeState::Ended { armed, .. } => armed.take(),
        _ => match std::mem::replace(
            &mut inner.state,
            TakeState::Idle {
                stream_denied: false,
            },
        ) {
            TakeState::Armed(armed) => Some(armed),
            _ => None,
        },
    }
    .expect("the state had an armed input");
    leave(
        &mut inner,
        TakeState::Recording {
            armed,
            take: Take {
                meta: TakeMeta {
                    project,
                    stem: stem.clone(),
                    start: fields,
                    input,
                    created_at: format_rfc3339_utc(created_at),
                },
                path,
                started: Instant::now(),
            },
        },
    );
    core.emit(StateEvent::new(RecState::Recording).stem(stem.clone()));
    Ok(StartResult { stem })
}

/// Stops the take and publishes it into the project it started in, with the webview's
/// `fields` (`None`: the fallback sidecar). For a take Rust already finalized (cap, device
/// loss) it replaces that take's sidecar instead. `other` with nothing to stop.
///
/// A take whose publish fails is kept as an unpublished ended take (the state event carries
/// the error): another stop retries it, and arm, start, disarm, reload and quit publish it with
/// the fallback sidecar.
pub fn stop(core: &Core, fields: Option<StopFields>) -> Result<RecordingEntry, CommandError> {
    let mut inner = core.lock();
    let idle = || TakeState::Idle {
        stream_denied: false,
    };
    if inner.state.is_recording() {
        let TakeState::Recording { armed, take } = std::mem::replace(&mut inner.state, idle())
        else {
            unreachable!("the state is recording");
        };
        let Some(recorder) = &inner.recorder else {
            inner.state = TakeState::Armed(armed);
            return Err(CommandError::other(NOT_ARMED));
        };
        let result = match recorder.stop() {
            Ok(result) => result,
            Err(e) => {
                // The take is dropped; its partial is left to the next open's repair.
                inner.state = TakeState::Armed(armed);
                eprintln!("recording: stopping {} failed: {e}", take.meta.stem);
                return Err(CommandError::from(e));
            }
        };
        inner.state = TakeState::Ended {
            armed: (!matches!(result.ended, Ended::DeviceLost(_))).then_some(armed),
            ended: EndedTake {
                meta: take.meta,
                result,
                published: false,
            },
        };
    }
    // The ended take is moved out, so a successful publish leaves the state without it.
    let (armed, mut ended) = match std::mem::replace(&mut inner.state, idle()) {
        TakeState::Ended { armed, ended } => (armed, ended),
        other => {
            inner.state = other;
            return Err(CommandError::other(NO_TAKE));
        }
    };
    let state = if armed.is_some() {
        RecState::Armed
    } else {
        RecState::Idle
    };
    inner.state = armed.map(TakeState::Armed).unwrap_or_else(idle);
    let stem = ended.meta.stem.clone();
    match complete_ended(&mut ended, fields.as_ref()) {
        Ok(entry) => {
            core.emit(StateEvent::new(state).stem(stem));
            Ok(entry)
        }
        Err(e) => {
            if !ended.published {
                eprintln!("recording: publishing {stem} failed: {}", e.message);
                let armed = match std::mem::replace(&mut inner.state, idle()) {
                    TakeState::Armed(armed) => Some(armed),
                    _ => None,
                };
                inner.state = TakeState::Ended { armed, ended };
            }
            core.emit(StateEvent::new(state).stem(stem).error(e.clone()));
            Err(e)
        }
    }
}

pub fn list(projects: &ProjectState, root: &str) -> Result<Vec<RecordingEntry>, CommandError> {
    project_for(projects, root)?
        .recordings()
        .map_err(CommandError::for_memos)
}

pub fn rename(
    projects: &ProjectState,
    root: &str,
    stem: &str,
    new_stem: &str,
) -> Result<String, CommandError> {
    project_for(projects, root)?
        .rename_recording(stem, new_stem)
        .map_err(CommandError::for_memos)
}

pub fn delete(projects: &ProjectState, root: &str, stem: &str) -> Result<(), CommandError> {
    project_for(projects, root)?
        .delete_recording(stem)
        .map_err(CommandError::for_memos)
}

pub fn read(projects: &ProjectState, root: &str, stem: &str) -> Result<Vec<u8>, CommandError> {
    project_for(projects, root)?
        .read_recording_bytes(stem)
        .map_err(CommandError::for_memos)
}

// --- reload, quit and project open ---

/// Stops a running take and publishes it with the fallback sidecar, and publishes an ended
/// take not published yet (logging failures).
fn finalize_running(inner: &mut Inner) {
    settle_ended(inner);
    if !inner.state.is_recording() {
        return;
    }
    let idle = TakeState::Idle {
        stream_denied: false,
    };
    let TakeState::Recording { armed, take } = std::mem::replace(&mut inner.state, idle) else {
        unreachable!("the state is recording");
    };
    // The caller clears the armed input next.
    inner.state = TakeState::Armed(armed);
    let Some(recorder) = &inner.recorder else {
        return;
    };
    match recorder.stop() {
        Ok(result) => {
            publish_fallback(&take.meta, &result);
        }
        Err(e) => eprintln!("recording: stopping {} failed: {e}", take.meta.stem),
    }
}

/// The webview is reloading: a take in progress is finalized with the fallback sidecar and the
/// recorder is disarmed, without events (the new page asks `recording_status`). Idle is a no-op.
pub fn on_reload(core: &Core) {
    let mut inner = core.lock();
    if matches!(inner.state, TakeState::Idle { .. }) {
        return;
    }
    finalize_running(&mut inner);
    if let Some(recorder) = &inner.recorder {
        recorder.disarm();
    }
    inner.state = TakeState::Idle {
        stream_denied: false,
    };
}

/// The app quits: finalizes a take in progress with the fallback sidecar, then shuts the
/// recorder down and joins it and the event forwarder. Emits nothing.
pub fn shutdown(core: &Core) {
    let (recorder, forwarder) = {
        let mut inner = core.lock();
        finalize_running(&mut inner);
        if !matches!(inner.state, TakeState::Idle { .. }) {
            inner.state = TakeState::Idle {
                stream_denied: false,
            };
        }
        (inner.recorder.take(), inner.forwarder.take())
    };
    // Outside the lock: the forwarder may be waiting for it.
    if let Some(recorder) = recorder {
        recorder.shutdown();
    }
    if let Some(forwarder) = forwarder
        && forwarder.join().is_err()
    {
        eprintln!("recording: the event forwarder panicked");
    }
}

/// What [`repair_on_open`] did.
#[derive(Debug, PartialEq)]
pub enum RepairOutcome {
    /// A take runs or just ended in this project: its partial is not touched.
    Skipped,
    Repaired(strudeland_project::RepairReport),
    /// Logged; opening the project does not fail because of it.
    Failed(String),
}

/// Recovers the `.partial` files a crash left in `project`, unless a take of this recorder
/// is in progress or ended there.
pub fn repair_on_open(core: &Core, project: &Project) -> RepairOutcome {
    {
        let inner = core.lock();
        let here = |p: &Project| p.root() == project.root();
        let held = match &inner.state {
            TakeState::Recording { take, .. } => here(&take.meta.project),
            TakeState::Ended { ended, .. } => here(&ended.meta.project),
            TakeState::Idle { .. } | TakeState::Armed(_) => false,
        };
        if held {
            return RepairOutcome::Skipped;
        }
    }
    match project.repair_partials() {
        Ok(report) => RepairOutcome::Repaired(report),
        Err(e) => {
            eprintln!(
                "recording: repairing {} failed: {e}",
                project.root().display()
            );
            RepairOutcome::Failed(e.to_string())
        }
    }
}

/// [`repair_on_open`] on a blocking thread, not awaited: the open command returns at once and
/// the renames reach the frontend as watcher events.
pub fn repair_in_background(core: &Arc<Core>, project: &Arc<Project>) {
    let core = Arc::clone(core);
    let project = Arc::clone(project);
    drop(tauri::async_runtime::spawn_blocking(move || {
        repair_on_open(&core, &project)
    }));
}

// --- the app's ports ---

/// The desktop microphone policy, where the OS exposes it.
mod permission {
    use super::Permission;

    #[cfg(target_os = "macos")]
    fn audio() -> &'static objc2_av_foundation::AVMediaType {
        // SAFETY: an immutable framework constant, set when AVFoundation loads.
        unsafe { objc2_av_foundation::AVMediaTypeAudio }.expect("AVMediaTypeAudio is defined")
    }

    /// `authorizationStatusForMediaType:` for audio; never prompts.
    #[cfg(target_os = "macos")]
    pub fn status() -> Permission {
        use objc2_av_foundation::AVCaptureDevice;
        // SAFETY: audio is a media type the method accepts.
        let raw = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio()) };
        Permission::from_raw(raw.0)
    }

    /// `requestAccessForMediaType:completionHandler:` for audio, blocking until the user
    /// answered. The handler runs on an arbitrary queue and only signals; the caller re-reads
    /// the status.
    #[cfg(target_os = "macos")]
    pub fn request() {
        use std::sync::Mutex;
        use std::sync::mpsc;

        use block2::RcBlock;
        use objc2::runtime::Bool;
        use objc2_av_foundation::AVCaptureDevice;

        let (tx, rx) = mpsc::channel::<()>();
        // The block is `Fn`: the sender is taken out on the first call.
        let tx = Mutex::new(Some(tx));
        let handler = RcBlock::new(move |_granted: Bool| {
            if let Some(tx) = tx.lock().ok().and_then(|mut t| t.take()) {
                let _ = tx.send(());
            }
        });
        // SAFETY: audio is a media type the method accepts; the block is retained by the call.
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio(), &handler) };
        let _ = rx.recv();
    }

    #[cfg(windows)]
    pub fn status() -> Permission {
        use windows::Devices::Enumeration::{
            DeviceAccessInformation, DeviceAccessStatus, DeviceClass,
        };
        match DeviceAccessInformation::CreateFromDeviceClass(DeviceClass::AudioCapture)
            .and_then(|info| info.CurrentStatus())
        {
            Ok(DeviceAccessStatus::Allowed) => Permission::Authorized,
            Ok(DeviceAccessStatus::DeniedByUser) => Permission::Denied,
            Ok(DeviceAccessStatus::DeniedBySystem) => Permission::Restricted,
            _ => Permission::NotDetermined,
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    pub fn status() -> Permission {
        Permission::NotDetermined
    }

    #[cfg(not(target_os = "macos"))]
    pub fn request() {}
}

/// The real OS: AVFoundation and cpal.
pub struct SystemPlatform;

impl Platform for SystemPlatform {
    fn has_permission_prompt(&self) -> bool {
        cfg!(target_os = "macos")
    }
    fn permission(&self) -> Permission {
        permission::status()
    }

    fn request_permission(&self) {
        permission::request();
    }

    fn devices(&self) -> Result<Vec<DeviceInfo>, RecordError> {
        strudeland_record::list_input_devices()
    }
}

/// Broadcasts the events to the webview.
pub struct AppSink(pub AppHandle);

impl Sink for AppSink {
    fn state(&self, event: StateEvent) {
        let _ = self.0.emit(STATE_EVENT, event);
    }

    fn level(&self, event: LevelEvent) {
        let _ = self.0.emit(LEVEL_EVENT, event);
    }
}

/// The managed state of the app.
pub fn system_state(app: &AppHandle) -> RecorderState {
    RecorderState::new(
        Box::new(SystemPlatform),
        Arc::new(AppSink(app.clone())),
        Box::new(Recorder::new),
    )
}

// --- Tauri commands ---

fn prefs_path(app: &AppHandle) -> Result<PathBuf, CommandError> {
    app.path()
        .app_data_dir()
        .map(|d| d.join(PREFS_FILE))
        .map_err(|e| CommandError::other(e.to_string()))
}

/// Runs `f` on a blocking thread with the app's states.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&Arc<Core>, &ProjectState) -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let core = Arc::clone(&app.state::<RecorderState>().0);
        let projects = app.state::<ProjectState>();
        f(&core, &projects)
    })
    .await
    .map_err(|e| CommandError::other(e.to_string()))?
}

#[tauri::command]
pub async fn recording_status(app: AppHandle) -> Result<RecordingStatus, CommandError> {
    let prefs = prefs_path(&app)?;
    blocking(app, move |core, _| Ok(status(core, &prefs))).await
}

#[tauri::command]
pub async fn recording_devices(app: AppHandle) -> Result<DevicesResult, CommandError> {
    let prefs = prefs_path(&app)?;
    blocking(app, move |core, _| devices(core, &prefs)).await
}

#[tauri::command]
pub async fn recording_arm(
    app: AppHandle,
    device_id: Option<String>,
    channel: Option<u16>,
) -> Result<RecordingStatus, CommandError> {
    let prefs = prefs_path(&app)?;
    blocking(app, move |core, _| arm(core, &prefs, device_id, channel)).await
}

#[tauri::command]
pub async fn recording_disarm(app: AppHandle) -> Result<RecordingStatus, CommandError> {
    let prefs = prefs_path(&app)?;
    blocking(app, move |core, _| disarm(core, &prefs)).await
}

#[tauri::command]
pub async fn recording_start(
    app: AppHandle,
    root: String,
    song_stem: Option<String>,
    fields: StartFields,
) -> Result<StartResult, CommandError> {
    blocking(app, move |core, projects| {
        start(core, projects, &root, song_stem.as_deref(), fields)
    })
    .await
}

/// `root` is accepted and ignored: the take finalizes into the project captured at start.
#[tauri::command]
pub async fn recording_stop(
    app: AppHandle,
    root: Option<String>,
    sidecar: Option<StopFields>,
) -> Result<RecordingEntry, CommandError> {
    let _ = root;
    blocking(app, move |core, _| stop(core, sidecar)).await
}

#[tauri::command]
pub async fn recording_list(
    app: AppHandle,
    root: String,
) -> Result<Vec<RecordingEntry>, CommandError> {
    blocking(app, move |_, projects| list(projects, &root)).await
}

#[tauri::command]
pub async fn recording_rename(
    app: AppHandle,
    root: String,
    stem: String,
    new_stem: String,
) -> Result<String, CommandError> {
    blocking(app, move |_, projects| {
        rename(projects, &root, &stem, &new_stem)
    })
    .await
}

#[tauri::command]
pub async fn recording_delete(
    app: AppHandle,
    root: String,
    stem: String,
) -> Result<(), CommandError> {
    blocking(app, move |_, projects| delete(projects, &root, &stem)).await
}

/// The WAV's raw bytes (an `ArrayBuffer` in the webview).
#[tauri::command]
pub async fn recording_read(
    app: AppHandle,
    root: String,
    stem: String,
) -> Result<tauri::ipc::Response, CommandError> {
    let bytes = blocking(app, move |_, projects| read(projects, &root, &stem)).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// The mach host time, a decimal string: the clock of the capture's anchors.
#[tauri::command]
pub async fn host_now_ns() -> String {
    strudeland_record::host_now_ns().to_string()
}

#[cfg(test)]
mod rig;
#[cfg(test)]
mod transcript;

#[cfg(test)]
mod tests {
    use super::rig::*;
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    use serde_json::json;
    use strudeland_record::FaultKind;
    use strudeland_record::testing::Limits;

    // --- errors and wire shapes ---

    #[test]
    fn project_for_is_typed_for_a_stale_root_and_no_project() {
        use CommandErrorKind as K;
        let rig = rig();
        let stale = format!("{}-old", rig.root);
        assert_eq!(
            project_for(&rig.projects, &stale).unwrap_err(),
            CommandError::new(K::ProjectChanged, "the project changed")
        );
        assert_eq!(
            project_for(&ProjectState::default(), &rig.root).unwrap_err(),
            CommandError::new(K::NoProject, "no project is open")
        );
    }

    #[test]
    fn project_errors_for_memos() {
        use CommandErrorKind as K;
        let cases = [
            (
                ProjectError::RecordingExists("a".into()),
                K::Exists,
                "memo a already exists",
            ),
            (
                ProjectError::RecordingNotFound("a".into()),
                K::NotFound,
                "memo a not found",
            ),
            (
                ProjectError::InvalidRecordingName("a/b".into()),
                K::InvalidName,
                "invalid memo name \"a/b\" (use letters, digits, space, - and _; 1 to 64 characters)",
            ),
            (
                ProjectError::SongExists("x".into()),
                K::Other,
                "song x already exists",
            ),
        ];
        for (error, kind, message) in cases {
            assert_eq!(
                CommandError::for_memos(error),
                CommandError::new(kind, message)
            );
        }
    }

    #[test]
    fn errors_from_record_errors() {
        use CommandErrorKind as K;
        let cases = [
            (RecordError::NoDevice, K::NoDevice, "no input device"),
            (RecordError::PermissionDenied, K::PermissionDenied, DENIED),
            #[cfg(target_os = "macos")]
            (
                RecordError::DeviceBusy,
                K::DeviceLost,
                "the input device is busy",
            ),
            #[cfg(not(target_os = "macos"))]
            (RecordError::DeviceBusy, K::Busy, "the input device is busy"),
            (
                RecordError::DeviceLost("gone".into()),
                K::DeviceLost,
                "the input device was lost: gone",
            ),
            (RecordError::AlreadyRecording, K::Busy, BUSY),
            (RecordError::NotArmed, K::Other, NOT_ARMED),
            (RecordError::StartTimeout, K::Other, NO_AUDIO),
            (
                RecordError::Backend("x".into()),
                K::Other,
                "audio backend: x",
            ),
        ];
        for (error, kind, message) in cases {
            let label = format!("{error:?}");
            assert_eq!(
                CommandError::from(error),
                CommandError::new(kind, message),
                "{label}"
            );
        }
    }

    #[test]
    fn a_busy_take_serializes_as_kind_and_message() {
        assert_eq!(
            serde_json::to_string(&CommandError::busy(BUSY)).unwrap(),
            r#"{"kind":"busy","message":"a take is in progress"}"#
        );
    }

    #[test]
    fn status_and_event_wire_shapes() {
        let status = RecordingStatus {
            state: RecState::Armed,
            permission: Permission::NotDetermined,
            device: Some(InputDevice::from(&usb())),
            channel: 3,
            stem: None,
            elapsed_seconds: None,
        };
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            r#"{"state":"armed","permission":"notDetermined","device":{"id":"fake:usb","name":"usb interface","channels":4,"sampleRate":1000,"isDefault":false,"bluetooth":false},"channel":3,"stem":null,"elapsedSeconds":null}"#
        );
        assert_eq!(
            serde_json::to_string(&StateEvent::new(RecState::Idle)).unwrap(),
            r#"{"state":"idle"}"#
        );
        let full = StateEvent::new(RecState::Armed)
            .stem("a-01")
            .error(CommandError::new(CommandErrorKind::LimitReached, LIMIT));
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            r#"{"state":"armed","stem":"a-01","error":{"kind":"limitReached","message":"recording stopped at the length limit"}}"#
        );
        assert_eq!(
            serde_json::to_string(&LevelEvent {
                peak: 0.5,
                rms: 0.25
            })
            .unwrap(),
            r#"{"peak":0.5,"rms":0.25}"#
        );
        assert_eq!(
            serde_json::to_string(&StartResult {
                stem: "m-01".into()
            })
            .unwrap(),
            r#"{"stem":"m-01"}"#
        );
        for (p, text) in [
            (Permission::Denied, "denied"),
            (Permission::Restricted, "restricted"),
            (Permission::Authorized, "authorized"),
        ] {
            assert_eq!(serde_json::to_string(&p).unwrap(), format!("\"{text}\""));
        }
    }

    #[test]
    fn stop_fields_parse_host_times_above_two_to_the_53() {
        let fields = stop_fields(Some("18446744073709551615"));
        assert_eq!(fields.timing.bar1_host_ns, Some(HostNs(u64::MAX)));
        assert_eq!(
            fields.timing.output_stamps,
            vec![(0.5, HostNs(1_000_000_123))]
        );
        let bad = serde_json::from_value::<StopFields>(json!({
            "output": { "baseLatency": 0, "outputLatency": null },
            "tempo": start_fields_json()["tempo"],
            "backing": start_fields_json()["backing"],
            "timing": { "bar1HostNs": 5, "clockOffsetNs": { "start": null, "end": null },
                        "outputStamps": [] },
        }));
        assert!(bad.is_err(), "a numeric bar1HostNs is rejected");
    }

    fn start_fields_json() -> serde_json::Value {
        json!({
            "tempo": { "bpm": 120, "meter": "4/4", "cps": 0.5, "beatsPerBar": 4,
                       "tempoOverride": false },
            "backing": { "song": false, "metronome": false, "countIn": false,
                         "songPath": null, "startSection": null, "loopSection": null },
        })
    }

    // --- permission ---

    #[test]
    fn raw_permission_values() {
        assert_eq!(Permission::from_raw(0), Permission::NotDetermined);
        assert_eq!(Permission::from_raw(1), Permission::Restricted);
        assert_eq!(Permission::from_raw(2), Permission::Denied);
        assert_eq!(Permission::from_raw(3), Permission::Authorized);
        assert_eq!(Permission::from_raw(4), Permission::Denied);
        assert_eq!(Permission::from_raw(-1), Permission::Denied);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_avfoundation_status_reads_without_prompting() {
        // Whatever this process's TCC state is, reading it neither prompts nor panics.
        let _ = permission::status();
    }

    #[test]
    fn the_bundle_declares_the_microphone() {
        let info = include_str!("../Info.plist");
        assert!(info.contains(
            "<key>NSMicrophoneUsageDescription</key>\n\t<string>strudeland records the microphone when you press record, to save voice memos in your project.</string>"
        ));
        let entitlements = include_str!("../Entitlements.plist");
        assert!(
            entitlements.contains("<key>com.apple.security.device.audio-input</key>\n\t<true/>")
        );
        assert!(!entitlements.contains("app-sandbox"));
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(
            conf["bundle"]["macOS"],
            json!({ "minimumSystemVersion": "13.0", "entitlements": "Entitlements.plist" })
        );
    }

    #[test]
    fn the_opener_scope_adds_only_the_microphone_settings_url() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let scoped: Vec<&serde_json::Value> = capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p.is_object())
            .collect();
        assert_eq!(
            scoped,
            vec![&json!({
                "identifier": "opener:allow-open-url",
                "allow": [{
                    "url": "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
                }, { "url": "ms-settings:privacy-microphone" }]
            })]
        );
    }

    #[test]
    fn not_determined_asks_then_arms_when_granted() {
        let rig = rig();
        *rig.platform.permission.lock().unwrap() = Permission::NotDetermined;
        let status = arm(&rig.core, &rig.prefs(), None, None).unwrap();
        assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 1);
        assert_eq!(status.state, RecState::Armed);
        assert_eq!(status.permission, Permission::Authorized);
        rig.next_handle();
        assert_eq!(rig.sink.states(), vec![StateEvent::new(RecState::Armed)]);
    }

    #[test]
    fn not_determined_asks_then_refuses_when_denied() {
        let rig = rig();
        *rig.platform.permission.lock().unwrap() = Permission::NotDetermined;
        *rig.platform.answer.lock().unwrap() = Permission::Denied;
        let error = arm(&rig.core, &rig.prefs(), None, None).unwrap_err();
        assert_eq!(
            error,
            CommandError::new(CommandErrorKind::PermissionDenied, DENIED)
        );
        assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 1);
        rig.no_new_handle();
        assert_eq!(
            rig.sink.states(),
            vec![StateEvent::new(RecState::Idle).error(error)]
        );
        assert_eq!(status(&rig.core, &rig.prefs()).state, RecState::Idle);
    }

    #[test]
    fn denied_and_restricted_open_no_stream() {
        for (permission, message) in [
            (Permission::Denied, DENIED),
            (Permission::Restricted, RESTRICTED),
        ] {
            let rig = rig();
            *rig.platform.permission.lock().unwrap() = permission;
            let error = arm(&rig.core, &rig.prefs(), None, None).unwrap_err();
            let expected = CommandError::new(CommandErrorKind::PermissionDenied, message);
            assert_eq!(error, expected, "{permission:?}");
            assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 0);
            rig.no_new_handle();
            assert_eq!(
                rig.sink.states(),
                vec![StateEvent::new(RecState::Idle).error(expected)]
            );
        }
    }

    #[test]
    fn a_second_arm_during_a_pending_request_is_busy() {
        let rig = rig();
        *rig.platform.permission.lock().unwrap() = Permission::NotDetermined;
        let (release, gate) = mpsc::channel();
        *rig.platform.gate.lock().unwrap() = Some(gate);
        std::thread::scope(|s| {
            let first = s.spawn(|| arm(&rig.core, &rig.prefs(), None, None));
            let deadline = Instant::now() + WAIT;
            while rig.platform.requests.load(Ordering::SeqCst) == 0 {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            let second = arm(&rig.core, &rig.prefs(), None, None).unwrap_err();
            assert_eq!(second, CommandError::busy(PERMISSION_PENDING));
            release.send(()).unwrap();
            assert_eq!(first.join().unwrap().unwrap().state, RecState::Armed);
        });
        assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 1);
        assert!(!rig.core.requesting.load(Ordering::SeqCst));
    }

    // --- arm and disarm ---

    #[test]
    fn arm_without_devices_is_no_device() {
        let rig = rig();
        rig.platform.devices.lock().unwrap().clear();
        let error = arm(&rig.core, &rig.prefs(), None, None).unwrap_err();
        assert_eq!(error.kind, CommandErrorKind::NoDevice);
        let error = arm(&rig.core, &rig.prefs(), Some("fake:gone".into()), None).unwrap_err();
        assert_eq!(error.kind, CommandErrorKind::NoDevice);
        rig.no_new_handle();
    }

    #[test]
    fn arm_with_an_absent_device_id_is_no_device() {
        let rig = rig();
        let error = arm(&rig.core, &rig.prefs(), Some("fake:gone".into()), None).unwrap_err();
        assert_eq!(
            error,
            CommandError::new(CommandErrorKind::NoDevice, "no input device")
        );
        assert!(!rig.prefs().exists());
    }

    #[test]
    fn on_a_desktop_without_a_prompt_the_stream_decides_access_and_can_be_retried() {
        let rig = rig();
        rig.platform.has_prompt.store(false, Ordering::Release);
        *rig.platform.permission.lock().unwrap() = Permission::NotDetermined;
        assert_eq!(
            status(&rig.core, &rig.prefs()).permission,
            Permission::NotDetermined
        );
        *rig.arm_failure.lock().unwrap() = Some(RecordError::PermissionDenied);
        let error = arm(&rig.core, &rig.prefs(), None, None).unwrap_err();
        assert_eq!(error.kind, CommandErrorKind::PermissionDenied);
        assert_eq!(
            status(&rig.core, &rig.prefs()).permission,
            Permission::Denied
        );
        let armed = arm(&rig.core, &rig.prefs(), None, None).unwrap();
        assert_eq!(armed.permission, Permission::Authorized);
        assert_eq!(rig.platform.requests.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_busy_device_stays_idle_and_reports_the_device_error() {
        let rig = rig();
        *rig.arm_failure.lock().unwrap() = Some(RecordError::DeviceBusy);
        let error = arm(&rig.core, &rig.prefs(), None, None).unwrap_err();
        #[cfg(target_os = "macos")]
        assert_eq!(error.kind, CommandErrorKind::DeviceLost);
        #[cfg(not(target_os = "macos"))]
        assert_eq!(error.kind, CommandErrorKind::Busy);
        assert_eq!(error.message, "the input device is busy");
        assert_eq!(status(&rig.core, &rig.prefs()).state, RecState::Idle);
        assert!(rig.sink.states().is_empty());
    }

    #[test]
    fn a_saved_exclusive_input_keeps_its_identity_and_armed_channel_count() {
        let rig = rig();
        let mut busy = usb();
        busy.channels = 0;
        busy.sample_rate = 0;
        *rig.platform.devices.lock().unwrap() = vec![mic(), busy];
        write_prefs(
            &rig.prefs(),
            &Prefs {
                device_id: Some(usb().id),
                channel: 2,
            },
        )
        .unwrap();

        // Enumeration could not open the USB endpoint. The stream-open error is authoritative.
        *rig.arm_failure.lock().unwrap() = Some(RecordError::PermissionDenied);
        let error = arm(&rig.core, &rig.prefs(), None, None).unwrap_err();
        assert_eq!(error.kind, CommandErrorKind::PermissionDenied);
        let st = arm(&rig.core, &rig.prefs(), None, None).unwrap();
        assert_eq!(st.device, Some(InputDevice::from(&usb())));
        assert_eq!(st.channel, 2);
        rig.next_handle();

        // With the exclusive stream open the picker still exposes the active channels.
        let list = devices(&rig.core, &rig.prefs()).unwrap();
        assert!(list.devices.contains(&InputDevice::from(&usb())));
        let st = arm(&rig.core, &rig.prefs(), Some(usb().id), Some(3)).unwrap();
        assert_eq!(st.channel, 3);
    }

    #[test]
    fn a_channel_out_of_range_is_other() {
        let rig = rig();
        let error = arm(&rig.core, &rig.prefs(), Some("fake:usb".into()), Some(4)).unwrap_err();
        assert_eq!(
            error,
            CommandError::other("channel 4 is out of range for 4 channels")
        );
        rig.no_new_handle();
    }

    #[test]
    fn arm_reports_the_armed_input_and_rearm_replaces_it() {
        let rig = rig();
        let status = arm(&rig.core, &rig.prefs(), None, None).unwrap();
        assert_eq!(status.device, Some(InputDevice::from(&mic())));
        assert_eq!(status.channel, 0);
        let first = rig.next_handle();
        let status = arm(&rig.core, &rig.prefs(), Some("fake:usb".into()), Some(2)).unwrap();
        assert_eq!(status.device, Some(InputDevice::from(&usb())));
        assert_eq!(status.channel, 2);
        let second = rig.next_handle();
        assert!(!first.is_open());
        assert!(second.is_open());
    }

    #[test]
    fn arm_and_disarm_during_a_take_are_busy() {
        let rig = rig();
        let handle = rig.take();
        assert_eq!(
            arm(&rig.core, &rig.prefs(), None, None).unwrap_err(),
            CommandError::busy(BUSY)
        );
        assert_eq!(
            disarm(&rig.core, &rig.prefs()).unwrap_err(),
            CommandError::busy(BUSY)
        );
        assert!(handle.is_recording());
    }

    #[test]
    fn disarm_closes_the_stream_and_emits_idle() {
        let rig = rig();
        let handle = rig.arm();
        let status = disarm(&rig.core, &rig.prefs()).unwrap();
        assert_eq!(status.state, RecState::Idle);
        assert_eq!(status.device, None);
        assert!(!handle.is_open());
        assert_eq!(rig.sink.last(), Some(StateEvent::new(RecState::Idle)));
        // Disarming again is harmless.
        assert_eq!(
            disarm(&rig.core, &rig.prefs()).unwrap().state,
            RecState::Idle
        );
    }

    #[test]
    fn levels_are_forwarded_while_armed() {
        let rig = rig();
        let handle = rig.arm();
        let deadline = Instant::now() + WAIT;
        while rig.sink.levels.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "no level event");
            handle.push(&[0.5; 10], T0);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // --- start ---

    #[test]
    fn start_names_the_take_and_reports_recording() {
        let rig = rig();
        let handle = rig.arm();
        assert_eq!(rig.start(&handle, Some("main")).unwrap().stem, "main-01");
        assert_eq!(
            rig.sink.last(),
            Some(StateEvent::new(RecState::Recording).stem("main-01"))
        );
        let status = status(&rig.core, &rig.prefs());
        assert_eq!(status.state, RecState::Recording);
        assert_eq!(status.stem.as_deref(), Some("main-01"));
        assert!(status.elapsed_seconds.is_some_and(|s| s >= 0.0));
        assert_eq!(rig.files(), vec![".main-01.wav.partial".to_string()]);
        stop(&rig.core, None).unwrap();
        assert_eq!(rig.start(&handle, Some("main")).unwrap().stem, "main-02");
        stop(&rig.core, None).unwrap();
        assert_eq!(rig.start(&handle, None).unwrap().stem, "memo-01");
        stop(&rig.core, None).unwrap();
        // Not a valid stem: the generic name.
        assert_eq!(rig.start(&handle, Some("a/b")).unwrap().stem, "memo-02");
    }

    #[test]
    fn start_while_recording_is_busy() {
        let rig = rig();
        let handle = rig.take();
        assert_eq!(
            rig.start(&handle, None).unwrap_err(),
            CommandError::busy(BUSY)
        );
    }

    #[test]
    fn start_unarmed_is_other() {
        let rig = rig();
        let error = start(&rig.core, &rig.projects, &rig.root, None, start_fields()).unwrap_err();
        assert_eq!(error, CommandError::other(NOT_ARMED));
        assert!(rig.files().is_empty());
    }

    #[test]
    fn start_with_a_stale_root_touches_nothing() {
        let rig = rig();
        let handle = rig.arm();
        let stale = format!("{}-old", rig.root);
        let error = start(&rig.core, &rig.projects, &stale, None, start_fields()).unwrap_err();
        assert_eq!(error, CommandError::project_changed());
        assert!(rig.files().is_empty());
        assert!(!handle.is_recording());
        assert_eq!(status(&rig.core, &rig.prefs()).state, RecState::Armed);
    }

    #[test]
    fn a_start_without_audio_times_out_and_leaves_no_partial() {
        let rig = rig_with(Limits {
            max_frames: None,
            start_timeout: Duration::from_millis(100),
        });
        rig.arm();
        let error = start(&rig.core, &rig.projects, &rig.root, None, start_fields()).unwrap_err();
        assert_eq!(error, CommandError::other(NO_AUDIO));
        assert!(rig.files().is_empty());
        assert_eq!(status(&rig.core, &rig.prefs()).state, RecState::Armed);
        assert_eq!(rig.sink.last(), Some(StateEvent::new(RecState::Armed)));
    }

    // --- stop ---

    #[test]
    fn stop_writes_the_webview_fields_input_and_anchors() {
        let rig = rig();
        rig.take();
        let entry = stop(&rig.core, Some(stop_fields(Some("1500000000")))).unwrap();
        assert_eq!(entry.stem, "main-01");
        assert_eq!(entry.frames, Some(1_000));
        assert_eq!(entry.sample_rate, Some(RATE));
        assert!(entry.has_sidecar);
        assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
        let sidecar = sidecar_of(&rig, "main-01");
        assert_eq!(entry.sidecar.as_ref(), Some(&sidecar));
        let fields = stop_fields(Some("1500000000"));
        assert_eq!(sidecar.schema, 1);
        assert_eq!(sidecar.app.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(sidecar.created_at.len(), "2026-01-01T00:00:00Z".len());
        assert_eq!(
            sidecar.input,
            SidecarInput {
                device_id: "fake:mic".into(),
                device_name: "fake mic".into(),
                channel: 0,
                channels: 1,
                sample_rate: RATE,
            }
        );
        assert_eq!(sidecar.output, fields.output);
        assert_eq!(sidecar.tempo, fields.tempo);
        assert_eq!(sidecar.backing, fields.backing);
        assert_eq!(sidecar.timing.bar1_host_ns, Some(HostNs(1_500_000_000)));
        // The first anchor is frame 0 at T0; 500 ms later at 1 kHz is frame 500.
        assert_eq!(sidecar.timing.bar1_frame, Some(500.0));
        assert_eq!(sidecar.timing.anchors.first(), Some(&(0, HostNs(T0))));
        assert_eq!(
            sidecar.timing.clock_offset_ns,
            fields.timing.clock_offset_ns
        );
        assert_eq!(sidecar.timing.output_stamps, fields.timing.output_stamps);
        assert_eq!(sidecar.xruns, 0);
        assert!(!sidecar.recovered);
        assert_eq!(
            rig.sink.last(),
            Some(StateEvent::new(RecState::Armed).stem("main-01"))
        );
        assert_eq!(status(&rig.core, &rig.prefs()).state, RecState::Armed);
    }

    #[test]
    fn stop_without_fields_writes_the_fallback_sidecar() {
        let rig = rig();
        rig.take();
        stop(&rig.core, None).unwrap();
        let sidecar = sidecar_of(&rig, "main-01");
        let start = start_fields();
        assert_eq!(
            sidecar.output,
            SidecarOutput {
                base_latency: 0.0,
                output_latency: None
            }
        );
        assert_eq!(sidecar.tempo, start.tempo);
        assert_eq!(sidecar.backing, start.backing);
        assert_eq!(sidecar.timing.bar1_frame, None);
        assert_eq!(sidecar.timing.bar1_host_ns, None);
        assert_eq!(sidecar.timing.clock_offset_ns.start, None);
        assert_eq!(sidecar.timing.clock_offset_ns.end, None);
        assert!(sidecar.timing.output_stamps.is_empty());
        assert!(!sidecar.timing.anchors.is_empty());
    }

    #[test]
    fn invalid_webview_fields_keep_the_audio_with_the_fallback() {
        let rig = rig();
        rig.take();
        let mut fields = stop_fields(Some("1500000000"));
        fields.tempo.meter = "four".into();
        let entry = stop(&rig.core, Some(fields)).unwrap();
        let sidecar = entry.sidecar.unwrap();
        assert_eq!(sidecar.tempo, start_fields().tempo);
        assert_eq!(sidecar.timing.bar1_host_ns, None);
    }

    #[test]
    fn a_bar1_outside_the_take_is_stored_null() {
        let rig = rig();
        rig.take();
        // 0.5 s before the first frame.
        let entry = stop(&rig.core, Some(stop_fields(Some("500000000")))).unwrap();
        let timing = entry.sidecar.unwrap().timing;
        assert_eq!(timing.bar1_host_ns, Some(HostNs(500_000_000)));
        assert_eq!(timing.bar1_frame, None);
    }

    #[test]
    fn a_second_stop_is_other() {
        let rig = rig();
        rig.take();
        stop(&rig.core, None).unwrap();
        assert_eq!(
            stop(&rig.core, None).unwrap_err(),
            CommandError::other(NO_TAKE)
        );
        let fresh = self::rig();
        assert_eq!(
            stop(&fresh.core, None).unwrap_err(),
            CommandError::other(NO_TAKE)
        );
    }

    #[test]
    fn stop_finalizes_into_the_project_captured_at_start() {
        let rig = rig();
        rig.take();
        let first = rig.project();
        let beta = rig
            .projects
            .open_unwatched(Project::create(&rig.tmp.path().join("beta"), "beta").unwrap());
        let entry = stop(&rig.core, None).unwrap();
        assert_eq!(entry.stem, "main-01");
        assert!(first.recording_wav_path("main-01").is_file());
        let second = project_for(&rig.projects, &beta).unwrap();
        assert!(second.recordings().unwrap().is_empty());
    }

    #[test]
    fn a_failed_publish_keeps_the_take_and_a_retry_publishes_it() {
        let rig = rig();
        rig.take();
        let conflict = block_publication(&rig);
        let error = stop(&rig.core, Some(stop_fields(Some("1500000000")))).unwrap_err();
        assert_eq!(rig.files(), vec![".main-01.wav.partial", "main-01.json"]);
        assert_eq!(std::fs::read(&conflict).unwrap(), b"existing sidecar");
        let event = rig.sink.last().unwrap();
        assert_eq!(
            (event.state, event.stem.as_deref(), event.error.as_ref()),
            (RecState::Armed, Some("main-01"), Some(&error))
        );
        assert_eq!(status(&rig.core, &rig.prefs()).state, RecState::Armed);
        // The conflict still exists, and the take remains available for retry.
        assert!(stop(&rig.core, None).is_err());
        std::fs::remove_file(conflict).unwrap();
        let entry = stop(&rig.core, Some(stop_fields(Some("1500000000")))).unwrap();
        assert_eq!(entry.stem, "main-01");
        assert_eq!(entry.frames, Some(1_000));
        assert_eq!(entry.sidecar.unwrap().timing.bar1_frame, Some(500.0));
        assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
        assert_eq!(
            rig.sink.last(),
            Some(StateEvent::new(RecState::Armed).stem("main-01"))
        );
        assert_eq!(
            stop(&rig.core, None).unwrap_err(),
            CommandError::other(NO_TAKE)
        );
    }

    #[test]
    fn a_take_that_failed_to_publish_is_published_by_disarm_reload_and_quit() {
        type Settle = fn(&Rig);
        let settles: [(Settle, &str); 3] = [
            (|r| drop(disarm(&r.core, &r.prefs()).unwrap()), "disarm"),
            (|r| on_reload(&r.core), "reload"),
            (|r| shutdown(&r.core), "quit"),
        ];
        for (settle, name) in settles {
            let rig = rig();
            rig.take();
            let conflict = block_publication(&rig);
            assert!(stop(&rig.core, None).is_err(), "{name}");
            std::fs::remove_file(conflict).unwrap();
            settle(&rig);
            assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"], "{name}");
            assert_eq!(sidecar_of(&rig, "main-01").timing.bar1_host_ns, None);
        }
    }

    #[test]
    fn leaving_an_unpublished_ended_take_publishes_it() {
        let rig = rig();
        rig.take();
        let conflict = block_publication(&rig);
        assert!(stop(&rig.core, None).is_err());
        let mut inner = rig.core.lock();
        assert!(matches!(inner.state, TakeState::Ended { .. }));
        std::fs::remove_file(conflict).unwrap();
        leave(
            &mut inner,
            TakeState::Idle {
                stream_denied: false,
            },
        );
        assert!(matches!(
            inner.state,
            TakeState::Idle {
                stream_denied: false
            }
        ));
        drop(inner);
        assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
        assert_eq!(sidecar_of(&rig, "main-01").timing.bar1_host_ns, None);
    }

    #[test]
    fn leaving_a_published_ended_take_writes_nothing() {
        let rig = rig_with(Limits {
            max_frames: Some(100),
            start_timeout: Duration::from_secs(2),
        });
        let handle = rig.arm();
        rig.start(&handle, Some("main")).unwrap();
        assert!(handle.push(&[0.5; 200], T0 + 10_000_000));
        rig.sink.wait(|e| e.error.is_some());
        let sidecar = rig.recordings_dir().join("main-01.json");
        std::fs::write(&sidecar, b"marker").unwrap();
        let mut inner = rig.core.lock();
        assert!(matches!(inner.state, TakeState::Ended { .. }));
        settle_ended(&mut inner);
        assert!(matches!(inner.state, TakeState::Armed(_)));
        drop(inner);
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"marker");
    }

    #[test]
    fn leaving_a_state_that_is_not_ended_publishes_nothing() {
        let rig = rig();
        rig.arm();
        let mut inner = rig.core.lock();
        assert!(matches!(inner.state, TakeState::Armed(_)));
        leave(
            &mut inner,
            TakeState::Idle {
                stream_denied: false,
            },
        );
        drop(inner);
        assert!(rig.files().is_empty());
    }

    #[test]
    fn an_invalid_start_tempo_still_saves_the_memo() {
        let rig = rig();
        let handle = rig.arm();
        let mut fields = start_fields();
        // `setcps(0)`.
        fields.tempo.cps = 0.0;
        rig.start_with(&handle, Some("main"), fields).unwrap();
        rig.push(&handle, 10, 1_000);
        let entry = stop(&rig.core, None).unwrap();
        let sidecar = entry.sidecar.unwrap();
        assert_eq!(sidecar.tempo, SidecarTempo::placeholder());
        assert_eq!(sidecar.backing, start_fields().backing);
        assert_eq!(sidecar.timing.bar1_frame, None);

        // Invalid webview fields fall back to the same sanitized sidecar.
        let mut fields = start_fields();
        fields.tempo.cps = 0.0;
        rig.start_with(&handle, Some("main"), fields).unwrap();
        rig.push(&handle, 10, 1_000);
        let mut stop_with = stop_fields(Some("1500000000"));
        stop_with.tempo.cps = 0.0;
        let entry = stop(&rig.core, Some(stop_with)).unwrap();
        assert_eq!(entry.stem, "main-02");
        assert_eq!(entry.sidecar.unwrap().tempo, SidecarTempo::placeholder());
    }

    // --- endings Rust handles by itself ---

    #[test]
    fn the_length_cap_finalizes_and_a_later_stop_is_idempotent() {
        let rig = rig_with(Limits {
            max_frames: Some(100),
            start_timeout: Duration::from_secs(2),
        });
        let handle = rig.arm();
        rig.start(&handle, Some("main")).unwrap();
        assert!(handle.push(&[0.5; 200], T0 + 10_000_000));
        let event = rig.sink.wait(|e| e.error.is_some());
        assert_eq!(
            event,
            StateEvent::new(RecState::Armed)
                .stem("main-01")
                .error(CommandError::new(CommandErrorKind::LimitReached, LIMIT))
        );
        let entry = rig.project().recordings().unwrap().remove(0);
        assert_eq!(entry.frames, Some(100));
        assert_eq!(entry.sidecar.unwrap().timing.bar1_host_ns, None);
        let status = status(&rig.core, &rig.prefs());
        assert_eq!(status.state, RecState::Armed);
        assert!(handle.is_open());
        // The webview's stop replaces the fallback sidecar.
        let entry = stop(&rig.core, Some(stop_fields(Some("1050000000")))).unwrap();
        let timing = entry.sidecar.unwrap().timing;
        assert_eq!(timing.bar1_host_ns, Some(HostNs(1_050_000_000)));
        assert_eq!(timing.bar1_frame, Some(50.0));
        assert_eq!(
            rig.sink.last(),
            Some(StateEvent::new(RecState::Armed).stem("main-01"))
        );
        assert_eq!(
            stop(&rig.core, None).unwrap_err(),
            CommandError::other(NO_TAKE)
        );
    }

    #[test]
    fn a_device_loss_finalizes_and_emits_idle() {
        let rig = rig();
        let handle = rig.take();
        assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
        let event = rig.sink.wait(|e| e.error.is_some());
        assert_eq!(event.state, RecState::Idle);
        assert_eq!(event.stem.as_deref(), Some("main-01"));
        assert_eq!(
            event.error.unwrap(),
            CommandError::new(
                CommandErrorKind::DeviceLost,
                "the input device was lost: unplugged"
            )
        );
        assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
        let fallback = sidecar_of(&rig, "main-01");
        assert_eq!(fallback.timing.bar1_frame, None);
        assert_eq!(fallback.timing.bar1_host_ns, None);
        let status = status(&rig.core, &rig.prefs());
        assert_eq!(status.state, RecState::Idle);
        assert_eq!(status.device, None);
        let entry = stop(&rig.core, Some(stop_fields(Some("1250000000")))).unwrap();
        assert_eq!(entry.sidecar.unwrap().timing.bar1_frame, Some(250.0));
        assert_eq!(
            rig.sink.last(),
            Some(StateEvent::new(RecState::Idle).stem("main-01"))
        );
        assert_eq!(
            stop(&rig.core, None).unwrap_err(),
            CommandError::other(NO_TAKE)
        );
        // The recorder arms again.
        rig.arm();
    }

    #[test]
    fn a_device_loss_while_armed_emits_idle_without_a_stem() {
        let rig = rig();
        let handle = rig.arm();
        assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
        let event = rig.sink.wait(|e| e.error.is_some());
        assert_eq!(event.state, RecState::Idle);
        assert_eq!(event.stem, None);
        assert_eq!(event.error.unwrap().kind, CommandErrorKind::DeviceLost);
    }

    #[test]
    fn a_stale_failed_after_a_rearm_is_ignored() {
        let rig = rig();
        rig.arm();
        let current = armed_id(&rig.core).unwrap();
        let events = rig.sink.states().len();
        // The `Failed` of an earlier stream, processed after this arm.
        failed(
            &rig.core,
            current - 1,
            RecordError::DeviceLost("old".into()),
        );
        assert_eq!(rig.sink.states().len(), events);
        let status = status(&rig.core, &rig.prefs());
        assert_eq!(status.state, RecState::Armed);
        assert!(status.device.is_some());
        // This stream's own `Failed` disarms.
        failed(&rig.core, current, RecordError::DeviceLost("now".into()));
        assert_eq!(rig.sink.last().unwrap().state, RecState::Idle);
        assert_eq!(super::status(&rig.core, &rig.prefs()).state, RecState::Idle);
    }

    #[test]
    fn a_rearm_after_a_device_loss_stays_armed() {
        let rig = rig();
        let handle = rig.arm();
        assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
        rig.sink.wait(|e| e.error.is_some());
        rig.arm();
        assert_eq!(status(&rig.core, &rig.prefs()).state, RecState::Armed);
    }

    #[test]
    fn arm_forgets_an_ended_take() {
        let rig = rig();
        let handle = rig.take();
        assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
        rig.sink.wait(|e| e.error.is_some());
        rig.arm();
        assert_eq!(
            stop(&rig.core, None).unwrap_err(),
            CommandError::other(NO_TAKE)
        );
    }

    // --- reload, quit and repair ---

    #[test]
    fn a_reload_finalizes_the_take_and_disarms_silently() {
        let rig = rig();
        let handle = rig.take();
        let events = rig.sink.states().len();
        on_reload(&rig.core);
        assert!(!handle.is_open());
        assert_eq!(rig.sink.states().len(), events, "no event on reload");
        let entry = list(&rig.projects, &rig.root).unwrap().remove(0);
        assert_eq!(entry.stem, "main-01");
        assert_eq!(entry.frames, Some(1_000));
        let timing = entry.sidecar.unwrap().timing;
        assert_eq!(timing.bar1_frame, None);
        assert_eq!(timing.bar1_host_ns, None);
        let status = status(&rig.core, &rig.prefs());
        assert_eq!(status.state, RecState::Idle);
        assert_eq!(
            stop(&rig.core, None).unwrap_err(),
            CommandError::other(NO_TAKE)
        );
        // The recorder arms again afterwards.
        rig.arm();
    }

    #[test]
    fn a_reload_while_idle_is_a_no_op() {
        let rig = rig();
        on_reload(&rig.core);
        assert!(rig.core.lock().recorder.is_none());
        assert!(rig.sink.states().is_empty());
        let handle = rig.arm();
        let events = rig.sink.states().len();
        on_reload(&rig.core);
        assert!(!handle.is_open(), "an armed stream is closed");
        assert_eq!(rig.sink.states().len(), events);
    }

    #[test]
    fn quit_finalizes_the_take_and_joins() {
        let rig = rig();
        let handle = rig.take();
        let events = rig.sink.states().len();
        shutdown(&rig.core);
        assert!(!handle.is_open());
        assert_eq!(rig.sink.states().len(), events, "no event on quit");
        {
            let inner = rig.core.lock();
            assert!(inner.recorder.is_none());
            assert!(inner.forwarder.is_none());
        }
        assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
        let sidecar = sidecar_of(&rig, "main-01");
        assert_eq!(sidecar.timing.bar1_frame, None);
        assert_eq!(sidecar.timing.clock_offset_ns.start, None);
        // Quitting twice, or idle, is harmless.
        shutdown(&rig.core);
        shutdown(&self::rig().core);
    }

    /// An empty partial last modified a minute ago (the repair discards it).
    fn stale_partial(project: &Project, stem: &str) -> PathBuf {
        let path = project
            .root()
            .join(format!("recordings/.{stem}.wav.partial"));
        let file = std::fs::File::create(&path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        path
    }

    #[test]
    fn repair_skips_a_project_with_a_take_in_progress_or_ended() {
        let rig = rig();
        let handle = rig.take();
        let project = rig.project();
        let old = stale_partial(&project, "memo-09");
        assert_eq!(repair_on_open(&rig.core, &project), RepairOutcome::Skipped);
        assert!(old.exists());
        assert!(handle.fault(FaultKind::DeviceLost, "unplugged"));
        rig.sink.wait(|e| e.error.is_some());
        assert_eq!(repair_on_open(&rig.core, &project), RepairOutcome::Skipped);
        assert!(old.exists());
        stop(&rig.core, None).unwrap();
        let RepairOutcome::Repaired(report) = repair_on_open(&rig.core, &project) else {
            panic!("not repaired");
        };
        assert_eq!(report.discarded, vec![".memo-09.wav.partial".to_string()]);
        assert!(!old.exists());
    }

    #[test]
    fn opening_a_project_repairs_it_in_the_background() {
        let rig = rig();
        let core = Arc::clone(&rig.core);
        rig.projects
            .on_open(move |project| repair_in_background(&core, project));
        let other = Project::create(&rig.tmp.path().join("beta"), "beta").unwrap();
        let old = stale_partial(&other, "memo-01");
        let dir = other.root().to_string_lossy().into_owned();
        let recents = rig.tmp.path().join("recent.json");
        crate::project::open_project(&rig.projects, &recents, &dir, |_| {
            Ok(crate::watcher::Watcher::inert())
        })
        .unwrap();
        let deadline = Instant::now() + WAIT;
        while old.exists() {
            assert!(Instant::now() < deadline, "the partial was not repaired");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn repair_runs_in_another_project_during_a_take() {
        let rig = rig();
        rig.take();
        let other = Project::create(&rig.tmp.path().join("beta"), "beta").unwrap();
        let old = stale_partial(&other, "memo-01");
        let RepairOutcome::Repaired(report) = repair_on_open(&rig.core, &other) else {
            panic!("not repaired");
        };
        assert_eq!(report.discarded, vec![".memo-01.wav.partial".to_string()]);
        assert!(!old.exists());
        assert!(rig.core.lock().state.is_recording(), "the take runs on");
    }

    #[test]
    fn a_repair_error_is_reported_not_raised() {
        let rig = rig();
        let other = Project::create(&rig.tmp.path().join("beta"), "beta").unwrap();
        let recordings = other.root().join("recordings");
        std::fs::remove_dir_all(&recordings).unwrap();
        std::fs::write(&recordings, "not a folder").unwrap();
        assert!(matches!(
            repair_on_open(&rig.core, &other),
            RepairOutcome::Failed(_)
        ));
    }

    #[test]
    fn list_and_stop_return_the_project_entry_camel_case() {
        let rig = rig();
        rig.take();
        let stopped = stop(&rig.core, None).unwrap();
        let listed = list(&rig.projects, &rig.root).unwrap();
        assert_eq!(listed, vec![stopped]);
        let value = serde_json::to_value(&listed[0]).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "duration",
                "frames",
                "hasSidecar",
                "modifiedMs",
                "sampleRate",
                "sidecar",
                "sidecarError",
                "size",
                "stem"
            ]
        );
        assert_eq!(value["stem"], "main-01");
        assert_eq!(value["frames"], 1_000);
        assert_eq!(value["sampleRate"], RATE);
        assert_eq!(value["duration"], 1.0);
        assert_eq!(value["hasSidecar"], true);
        assert_eq!(value["sidecarError"], serde_json::Value::Null);
        assert_eq!(
            value["sidecar"]["timing"]["bar1Frame"],
            serde_json::Value::Null
        );
        let wav = std::fs::read(rig.recordings_dir().join("main-01.wav")).unwrap();
        assert_eq!(value["size"], wav.len());
        // `recording_read` hands these bytes to `tauri::ipc::Response`, an ArrayBuffer in JS.
        assert_eq!(read(&rig.projects, &rig.root, "main-01").unwrap(), wav);
        assert_eq!(&wav[..4], b"RIFF");
    }

    // --- bar 1 ---

    fn a(frame: u64, host_ns: u64) -> Anchor {
        Anchor { frame, host_ns }
    }

    #[test]
    fn bar1_frame_cases() {
        let anchors = [a(0, T0), a(1_000, T0 + 1_000_000_000)];
        assert_eq!(bar1_frame(&anchors, RATE, 2_000, None), None);
        assert_eq!(bar1_frame(&[], RATE, 2_000, Some(T0)), None);
        // Before the first frame.
        assert_eq!(
            bar1_frame(&anchors, RATE, 2_000, Some(T0 - 1_000_000)),
            None
        );
        // Beyond the take's frames.
        assert_eq!(
            bar1_frame(&anchors, RATE, 1_500, Some(T0 + 1_600_000_000)),
            None
        );
        // Interior, both ends of the range.
        assert_eq!(
            bar1_frame(&anchors, RATE, 2_000, Some(T0 + 250_500_000)),
            Some(250.5)
        );
        assert_eq!(bar1_frame(&anchors, RATE, 2_000, Some(T0)), Some(0.0));
        assert_eq!(
            bar1_frame(&anchors, RATE, 1_500, Some(T0 + 1_500_000_000)),
            Some(1_500.0)
        );
    }

    #[test]
    fn bar1_frame_above_two_to_the_53() {
        let base = (1u64 << 60) + 7;
        let anchors = [a(0, base), a(48_000, base + 1_000_000_000)];
        assert_eq!(
            bar1_frame(&anchors, 48_000, 96_000, Some(base + 500_000_000)),
            Some(24_000.0)
        );
        assert_eq!(
            bar1_frame(&anchors, 48_000, 96_000, Some(base + 1)),
            Some(48_000.0 / 1e9)
        );
    }

    // --- prefs ---

    #[test]
    fn prefs_parse_errors_give_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(PREFS_FILE);
        assert_eq!(read_prefs(&path), Prefs::default());
        let cases: [(&str, &str); 6] = [
            ("{", "EOF while parsing an object at line 1 column 1"),
            (
                r#"{"schema":2,"deviceId":null,"channel":0}"#,
                "schema must be 1",
            ),
            (
                r#"{"deviceId":null,"channel":0}"#,
                "missing field `schema` at line 1 column 29",
            ),
            (
                r#"{"schema":1,"deviceId":3,"channel":0}"#,
                "invalid type: integer `3`, expected a string at line 1 column 24",
            ),
            (
                r#"{"schema":1,"deviceId":null,"channel":-1}"#,
                "invalid value: integer `-1`, expected u16 at line 1 column 40",
            ),
            (
                r#"{"schema":1,"deviceId":null,"channel":"0"}"#,
                "invalid type: string \"0\", expected u16 at line 1 column 41",
            ),
        ];
        for (text, message) in cases {
            assert_eq!(
                parse_prefs(text.as_bytes()),
                Err(message.to_string()),
                "{text}"
            );
            std::fs::write(&path, text).unwrap();
            assert_eq!(read_prefs(&path), Prefs::default(), "{text}");
            // The bad file is left in place.
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        // A directory where the file should be.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(read_prefs(&path), Prefs::default());
    }

    #[test]
    fn prefs_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("data").join(PREFS_FILE);
        let prefs = Prefs {
            device_id: Some("coreaudio:abc".into()),
            channel: 3,
        };
        write_prefs(&path, &prefs).unwrap();
        assert_eq!(read_prefs(&path), prefs);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"schema\": 1,\n  \"deviceId\": \"coreaudio:abc\",\n  \"channel\": 3\n}\n"
        );
        assert_eq!(
            parse_prefs(br#"{"schema":1,"channel":2}"#),
            Ok(Prefs {
                device_id: None,
                channel: 2
            })
        );
    }

    #[test]
    fn selection_resolution() {
        let devices = [mic(), usb()];
        let saved = |id: Option<&str>, channel| Prefs {
            device_id: id.map(str::to_string),
            channel,
        };
        let pick =
            |prefs: &Prefs| resolve_selection(&devices, prefs).map(|(d, c)| (d.id.clone(), c));
        assert_eq!(pick(&Prefs::default()), Some(("fake:mic".into(), 0)));
        assert_eq!(
            pick(&saved(Some("fake:usb"), 3)),
            Some(("fake:usb".into(), 3))
        );
        // Channel clamp.
        assert_eq!(
            pick(&saved(Some("fake:usb"), 4)),
            Some(("fake:usb".into(), 0))
        );
        // Saved device absent: the default input, the channel checked against it.
        assert_eq!(
            pick(&saved(Some("fake:gone"), 3)),
            Some(("fake:mic".into(), 0))
        );
        // No default flagged: the first device.
        let plain = [device("x", "x", 2, false), device("y", "y", 2, false)];
        assert_eq!(
            resolve_selection(&plain, &saved(None, 1)).map(|(d, c)| (d.id.as_str(), c)),
            Some(("x", 1))
        );
        assert_eq!(resolve_selection(&[], &Prefs::default()), None);
    }

    #[test]
    fn saved_channel_cases() {
        let saved = |channel| Prefs {
            device_id: None,
            channel,
        };
        // An exclusive endpoint reports no channels: any saved channel is kept.
        let exclusive = device("x", "x", 0, false);
        assert_eq!(saved_channel(&exclusive, &saved(7)), 7);
        let usb = usb();
        assert_eq!(saved_channel(&usb, &saved(3)), 3);
        assert_eq!(saved_channel(&usb, &saved(4)), 0);
        assert_eq!(saved_channel(&usb, &saved(9)), 0);
    }

    #[test]
    fn arm_writes_prefs_only_when_it_carried_an_argument() {
        let rig = rig();
        write_prefs(
            &rig.prefs(),
            &Prefs {
                device_id: Some("fake:gone".into()),
                channel: 2,
            },
        )
        .unwrap();
        let before = std::fs::read_to_string(rig.prefs()).unwrap();
        // The saved device is absent: the default input, and the file is not rewritten.
        let status = arm(&rig.core, &rig.prefs(), None, None).unwrap();
        assert_eq!(status.device.unwrap().id, "fake:mic");
        assert_eq!(status.channel, 0);
        assert_eq!(std::fs::read_to_string(rig.prefs()).unwrap(), before);
        arm(&rig.core, &rig.prefs(), None, Some(0)).unwrap();
        assert_eq!(
            read_prefs(&rig.prefs()),
            Prefs {
                device_id: Some("fake:mic".into()),
                channel: 0
            }
        );
        arm(&rig.core, &rig.prefs(), Some("fake:usb".into()), None).unwrap();
        assert_eq!(
            read_prefs(&rig.prefs()),
            Prefs {
                device_id: Some("fake:usb".into()),
                channel: 0
            }
        );
    }

    #[test]
    fn a_prefs_write_failure_does_not_fail_the_arm() {
        let rig = rig();
        std::fs::create_dir_all(rig.prefs()).unwrap();
        let status = arm(&rig.core, &rig.prefs(), Some("fake:usb".into()), Some(1)).unwrap();
        assert_eq!(status.state, RecState::Armed);
        assert_eq!(status.channel, 1);
    }

    #[test]
    fn devices_and_idle_status_use_the_saved_choice() {
        let rig = rig();
        let result = devices(&rig.core, &rig.prefs()).unwrap();
        assert_eq!(
            result.devices,
            vec![InputDevice::from(&mic()), InputDevice::from(&usb())]
        );
        assert_eq!(
            result.selected,
            Some(InputSelection {
                device_id: "fake:mic".into(),
                channel: 0
            })
        );
        write_prefs(
            &rig.prefs(),
            &Prefs {
                device_id: Some("fake:usb".into()),
                channel: 2,
            },
        )
        .unwrap();
        let result = devices(&rig.core, &rig.prefs()).unwrap();
        assert_eq!(
            result.selected,
            Some(InputSelection {
                device_id: "fake:usb".into(),
                channel: 2
            })
        );
        let idle = status(&rig.core, &rig.prefs());
        assert_eq!(
            idle,
            RecordingStatus {
                state: RecState::Idle,
                permission: Permission::Authorized,
                device: None,
                channel: 2,
                stem: None,
                elapsed_seconds: None,
            }
        );
        rig.platform.devices.lock().unwrap().clear();
        let none = devices(&rig.core, &rig.prefs()).unwrap();
        assert!(none.devices.is_empty());
        assert_eq!(none.selected, None);
    }

    // --- the memo library ---

    #[test]
    fn library_commands_reject_a_stale_root() {
        let rig = rig();
        rig.take();
        stop(&rig.core, None).unwrap();
        let stale = format!("{}-old", rig.root);
        let changed = CommandError::project_changed();
        assert_eq!(list(&rig.projects, &stale).unwrap_err(), changed);
        assert_eq!(
            rename(&rig.projects, &stale, "main-01", "x").unwrap_err(),
            changed
        );
        assert_eq!(
            delete(&rig.projects, &stale, "main-01").unwrap_err(),
            changed
        );
        assert_eq!(read(&rig.projects, &stale, "main-01").unwrap_err(), changed);
        assert_eq!(rig.files(), vec!["main-01.json", "main-01.wav"]);
    }

    #[test]
    fn library_commands_work_and_map_errors() {
        use CommandErrorKind as K;
        let rig = rig();
        let handle = rig.take();
        // A take's partial is never listed.
        assert!(list(&rig.projects, &rig.root).unwrap().is_empty());
        stop(&rig.core, None).unwrap();
        rig.start(&handle, Some("main")).unwrap();
        stop(&rig.core, None).unwrap();
        let stems: Vec<String> = list(&rig.projects, &rig.root)
            .unwrap()
            .into_iter()
            .map(|e| e.stem)
            .collect();
        assert_eq!(stems, vec!["main-01", "main-02"]);
        let wav = std::fs::read(rig.recordings_dir().join("main-01.wav")).unwrap();
        assert_eq!(read(&rig.projects, &rig.root, "main-01").unwrap(), wav);
        assert_eq!(
            rename(&rig.projects, &rig.root, "main-01", "main-02")
                .unwrap_err()
                .kind,
            K::Exists
        );
        assert_eq!(
            rename(&rig.projects, &rig.root, "main-01", "a/b")
                .unwrap_err()
                .kind,
            K::InvalidName
        );
        assert_eq!(
            rename(&rig.projects, &rig.root, "nope", "x")
                .unwrap_err()
                .kind,
            K::NotFound
        );
        assert_eq!(
            rename(&rig.projects, &rig.root, "main-01", "verse idea").unwrap(),
            "verse idea"
        );
        assert_eq!(
            read(&rig.projects, &rig.root, "main-01").unwrap_err().kind,
            K::NotFound
        );
        assert_eq!(read(&rig.projects, &rig.root, "verse idea").unwrap(), wav);
        delete(&rig.projects, &rig.root, "verse idea").unwrap();
        assert_eq!(
            delete(&rig.projects, &rig.root, "verse idea")
                .unwrap_err()
                .kind,
            K::NotFound
        );
        assert_eq!(rig.files(), vec!["main-02.json", "main-02.wav"]);
    }

    #[test]
    fn host_now_is_a_decimal_string() {
        let text = tauri::async_runtime::block_on(host_now_ns());
        assert!(text.parse::<u64>().is_ok(), "{text}");
    }
}
