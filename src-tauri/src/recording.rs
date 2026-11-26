//! Recording: the wire types, the Tauri commands and the app's ports over
//! `strudeland_record::takes` (the take lifecycle) and `strudeland-project` (the memo files in
//! `recordings/`).
//!
//! The logic lives in [`Takes`]. This file maps its domain types and [`TakeError`] to the wire
//! (`recording://state`, the command results, [`CommandError`]), implements its ports for the
//! app ([`SystemPlatform`]: the microphone permission and cpal's devices; [`AppSink`]: the
//! events), and holds the `#[tauri::command]` wrappers, which only resolve paths and move the
//! work onto a blocking thread; the state mutex is only held there, never across an `.await`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use strudeland_project::{
    HostNs, Project, RecordingEntry, SidecarBacking, SidecarClockOffsets, SidecarOutput,
    SidecarTempo,
};
use strudeland_record::takes::{
    Devices, Platform, Selection, Sink, TakeError, TakeEvent, TakeStart, TakeStatus, TakeStop,
    Takes, TakesConfig, repair_on_open,
};
pub use strudeland_record::takes::{Permission, RecState};
use strudeland_record::{DeviceInfo, Level, RecordError, Recorder};
use tauri::{AppHandle, Emitter, Manager};

use crate::command_error::{CommandError, CommandErrorKind};
use crate::project::{AppDataState, ProjectState};

/// `{peak, rms}`, about 30 Hz while armed or recording.
pub const LEVEL_EVENT: &str = "recording://level";
/// `{state, stem?, error?}` on every recorder transition.
pub const STATE_EVENT: &str = "recording://state";

// --- errors ---

/// A recorder error with no domain meaning, as the wire classifies it.
fn record_error(e: &RecordError) -> CommandError {
    use CommandErrorKind as K;
    match e {
        RecordError::NoDevice => CommandError::new(K::NoDevice, e.to_string()),
        // Keep the existing CoreAudio wire classification. On ALSA and WASAPI a busy
        // endpoint is common when another client holds it exclusively.
        #[cfg(target_os = "macos")]
        RecordError::DeviceBusy => CommandError::new(K::DeviceLost, e.to_string()),
        #[cfg(not(target_os = "macos"))]
        RecordError::DeviceBusy => CommandError::busy(e.to_string()),
        RecordError::DeviceLost(_) => CommandError::new(K::DeviceLost, e.to_string()),
        other => CommandError::other(other.to_string()),
    }
}

/// The one table of take errors on the wire; each message is the error's `Display`.
impl From<TakeError> for CommandError {
    fn from(e: TakeError) -> Self {
        use CommandErrorKind as K;
        match &e {
            TakeError::Busy | TakeError::PermissionPending => CommandError::busy(e.to_string()),
            TakeError::NotArmed
            | TakeError::NoTake
            | TakeError::NoAudio
            | TakeError::ChannelOutOfRange { .. }
            | TakeError::WriteFailed(_) => CommandError::other(e.to_string()),
            TakeError::PermissionDenied | TakeError::PermissionRestricted => {
                CommandError::new(K::PermissionDenied, e.to_string())
            }
            TakeError::LimitReached => CommandError::new(K::LimitReached, e.to_string()),
            TakeError::Record(error) => record_error(error),
            TakeError::Memo(error) => CommandError::for_memo_error(error),
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

impl From<TakeStatus> for RecordingStatus {
    fn from(s: TakeStatus) -> Self {
        RecordingStatus {
            state: s.state,
            permission: s.permission,
            device: s.device.as_ref().map(InputDevice::from),
            channel: s.channel,
            stem: s.stem,
            elapsed_seconds: s.elapsed_seconds,
        }
    }
}

/// A device and its 0-based channel.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InputSelection {
    pub device_id: String,
    pub channel: u16,
}

impl From<Selection> for InputSelection {
    fn from(s: Selection) -> Self {
        InputSelection {
            device_id: s.device_id,
            channel: s.channel,
        }
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DevicesResult {
    pub devices: Vec<InputDevice>,
    /// The saved choice resolved against the present devices; `None` without any input device.
    pub selected: Option<InputSelection>,
}

impl From<Devices> for DevicesResult {
    fn from(d: Devices) -> Self {
        DevicesResult {
            devices: d.devices.iter().map(InputDevice::from).collect(),
            selected: d.selected.map(InputSelection::from),
        }
    }
}

/// What `recording_start` takes: enough for a timing-null sidecar if the webview never stops.
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StartFields {
    pub tempo: SidecarTempo,
    pub backing: SidecarBacking,
}

impl From<StartFields> for TakeStart {
    fn from(f: StartFields) -> Self {
        TakeStart {
            tempo: f.tempo,
            backing: f.backing,
        }
    }
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

impl From<StopFields> for TakeStop {
    fn from(f: StopFields) -> Self {
        TakeStop {
            output: f.output,
            tempo: f.tempo,
            backing: f.backing,
            bar1_host_ns: f.timing.bar1_host_ns,
            clock_offset_ns: f.timing.clock_offset_ns,
            output_stamps: f.timing.output_stamps,
        }
    }
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

impl From<Level> for LevelEvent {
    fn from(level: Level) -> Self {
        LevelEvent {
            peak: level.peak,
            rms: level.rms,
        }
    }
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

impl From<TakeEvent> for StateEvent {
    fn from(e: TakeEvent) -> Self {
        StateEvent {
            state: e.state,
            stem: e.stem,
            error: e.error.map(Into::into),
        }
    }
}

// --- the memo library ---

pub fn list(projects: &ProjectState, root: &str) -> Result<Vec<RecordingEntry>, CommandError> {
    projects
        .current_for(root)?
        .recordings()
        .map_err(CommandError::for_memos)
}

pub fn rename(
    projects: &ProjectState,
    root: &str,
    stem: &str,
    new_stem: &str,
) -> Result<String, CommandError> {
    projects
        .current_for(root)?
        .rename_recording(stem, new_stem)
        .map_err(CommandError::for_memos)
}

pub fn delete(projects: &ProjectState, root: &str, stem: &str) -> Result<(), CommandError> {
    projects
        .current_for(root)?
        .delete_recording(stem)
        .map_err(CommandError::for_memos)
}

pub fn read(projects: &ProjectState, root: &str, stem: &str) -> Result<Vec<u8>, CommandError> {
    projects
        .current_for(root)?
        .read_recording_bytes(stem)
        .map_err(CommandError::for_memos)
}

/// [`repair_on_open`] on a blocking thread, not awaited: the open command returns at once and
/// the renames reach the frontend as watcher events.
pub fn repair_in_background(takes: &Arc<Takes>, project: &Arc<Project>) {
    let takes = Arc::clone(takes);
    let project = Arc::clone(project);
    drop(tauri::async_runtime::spawn_blocking(move || {
        repair_on_open(&takes, &project)
    }));
}

// --- the app's ports ---

/// The desktop microphone policy, where the OS exposes it.
mod permission {
    use strudeland_record::takes::Permission;

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
    fn state(&self, event: TakeEvent) {
        let _ = self.0.emit(STATE_EVENT, StateEvent::from(event));
    }

    fn level(&self, level: Level) {
        let _ = self.0.emit(LEVEL_EVENT, LevelEvent::from(level));
    }
}

/// The managed state: one [`Takes`], shared with the event forwarder thread.
pub struct RecorderState(pub Arc<Takes>);

/// What the take lifecycle needs from the app: its version, for the sidecar's `app.version`.
fn takes_config() -> TakesConfig {
    TakesConfig {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// The managed state of the app.
pub fn system_state(app: &AppHandle) -> RecorderState {
    RecorderState(Takes::new(
        Box::new(SystemPlatform),
        Arc::new(AppSink(app.clone())),
        Box::new(Recorder::new),
        takes_config(),
    ))
}

// --- Tauri commands ---

/// Runs `f` on a blocking thread with the app's states.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&Arc<Takes>, &ProjectState) -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let takes = Arc::clone(&app.state::<RecorderState>().0);
        let projects = app.state::<ProjectState>();
        f(&takes, &projects)
    })
    .await
    .map_err(|e| CommandError::other(e.to_string()))?
}

#[tauri::command]
pub async fn recording_status(app: AppHandle) -> Result<RecordingStatus, CommandError> {
    let app_data = app.state::<AppDataState>().get()?.clone();
    blocking(app, move |takes, _| Ok(takes.status(&app_data).into())).await
}

#[tauri::command]
pub async fn recording_devices(app: AppHandle) -> Result<DevicesResult, CommandError> {
    let app_data = app.state::<AppDataState>().get()?.clone();
    blocking(app, move |takes, _| {
        takes
            .devices(&app_data)
            .map(DevicesResult::from)
            .map_err(CommandError::from)
    })
    .await
}

#[tauri::command]
pub async fn recording_arm(
    app: AppHandle,
    device_id: Option<String>,
    channel: Option<u16>,
) -> Result<RecordingStatus, CommandError> {
    let app_data = app.state::<AppDataState>().get()?.clone();
    blocking(app, move |takes, _| {
        takes
            .arm(&app_data, device_id, channel)
            .map(RecordingStatus::from)
            .map_err(CommandError::from)
    })
    .await
}

#[tauri::command]
pub async fn recording_disarm(app: AppHandle) -> Result<RecordingStatus, CommandError> {
    let app_data = app.state::<AppDataState>().get()?.clone();
    blocking(app, move |takes, _| {
        takes
            .disarm(&app_data)
            .map(RecordingStatus::from)
            .map_err(CommandError::from)
    })
    .await
}

#[tauri::command]
pub async fn recording_start(
    app: AppHandle,
    root: String,
    song_stem: Option<String>,
    fields: StartFields,
) -> Result<StartResult, CommandError> {
    blocking(app, move |takes, projects| {
        takes
            .start(
                || projects.current_for(&root),
                song_stem.as_deref(),
                fields.into(),
            )
            .map(|stem| StartResult { stem })
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
    blocking(app, move |takes, _| {
        takes
            .stop(sidecar.map(TakeStop::from))
            .map_err(CommandError::from)
    })
    .await
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
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime};

    use serde_json::json;
    use strudeland_project::{AppData, ProjectError};

    // --- errors and wire shapes ---

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
    fn errors_map_to_command_errors() {
        use CommandErrorKind as K;
        let records = [
            (RecordError::NoDevice, K::NoDevice, "no input device"),
            (
                RecordError::PermissionDenied,
                K::PermissionDenied,
                "microphone access denied",
            ),
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
            (
                RecordError::UnsupportedFormat("f".into()),
                K::Other,
                "unsupported input format: f",
            ),
            (RecordError::NotArmed, K::Other, "the recorder is not armed"),
            (
                RecordError::AlreadyRecording,
                K::Busy,
                "a take is in progress",
            ),
            (
                RecordError::StartTimeout,
                K::Other,
                "no audio arrived from the input",
            ),
            (
                RecordError::Io(std::io::Error::other("disk")),
                K::Other,
                "disk",
            ),
            (RecordError::Wav("w".into()), K::Other, "wav: w"),
            (
                RecordError::Backend("x".into()),
                K::Other,
                "audio backend: x",
            ),
        ];
        for (error, kind, message) in records {
            let label = format!("{error:?}");
            assert_eq!(
                CommandError::from(TakeError::from(error)),
                CommandError::new(kind, message),
                "{label}"
            );
        }
        #[cfg(target_os = "macos")]
        let restricted = "microphone access is restricted on this mac";
        #[cfg(not(target_os = "macos"))]
        let restricted = "microphone access is restricted by the system";
        let takes = [
            (TakeError::Busy, K::Busy, "a take is in progress"),
            (
                TakeError::PermissionPending,
                K::Busy,
                "a microphone access request is pending",
            ),
            (TakeError::NotArmed, K::Other, "the recorder is not armed"),
            (TakeError::NoTake, K::Other, "no take in progress"),
            (
                TakeError::NoAudio,
                K::Other,
                "no audio arrived from the input",
            ),
            (
                TakeError::PermissionDenied,
                K::PermissionDenied,
                "microphone access denied",
            ),
            (
                TakeError::PermissionRestricted,
                K::PermissionDenied,
                restricted,
            ),
            (
                TakeError::ChannelOutOfRange {
                    channel: 4,
                    channels: 4,
                },
                K::Other,
                "channel 4 is out of range for 4 channels",
            ),
            (
                TakeError::LimitReached,
                K::LimitReached,
                "recording stopped at the length limit",
            ),
            (
                TakeError::WriteFailed("disk full".into()),
                K::Other,
                "the take could not be written: disk full",
            ),
            (
                TakeError::Record(Arc::new(RecordError::DeviceLost("gone".into()))),
                K::DeviceLost,
                "the input device was lost: gone",
            ),
            (
                TakeError::Memo(Arc::new(ProjectError::RecordingExists("a".into()))),
                K::Exists,
                "memo a already exists",
            ),
            (
                TakeError::Memo(Arc::new(ProjectError::SongExists("x".into()))),
                K::Other,
                "song x already exists",
            ),
        ];
        for (error, kind, message) in takes {
            let label = format!("{error:?}");
            assert_eq!(error.to_string(), message, "{label}");
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
            serde_json::to_string(&CommandError::from(TakeError::Busy)).unwrap(),
            r#"{"kind":"busy","message":"a take is in progress"}"#
        );
    }

    #[test]
    fn status_and_event_wire_shapes() {
        let status = RecordingStatus {
            state: RecState::Armed,
            permission: Permission::NotDetermined,
            device: Some(InputDevice::from(&FakePlatform::usb())),
            channel: 3,
            stem: None,
            elapsed_seconds: None,
        };
        assert_eq!(
            serde_json::to_string(&status).unwrap(),
            r#"{"state":"armed","permission":"notDetermined","device":{"id":"fake:usb","name":"usb interface","channels":4,"sampleRate":1000,"isDefault":false,"bluetooth":false},"channel":3,"stem":null,"elapsedSeconds":null}"#
        );
        let from_domain = RecordingStatus::from(TakeStatus {
            state: RecState::Armed,
            permission: Permission::NotDetermined,
            device: Some(FakePlatform::usb()),
            channel: 3,
            stem: None,
            elapsed_seconds: None,
        });
        assert_eq!(from_domain, status);
        let idle = StateEvent {
            state: RecState::Idle,
            stem: None,
            error: None,
        };
        assert_eq!(serde_json::to_string(&idle).unwrap(), r#"{"state":"idle"}"#);
        let full = StateEvent {
            state: RecState::Armed,
            stem: Some("a-01".into()),
            error: Some(TakeError::LimitReached.into()),
        };
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            r#"{"state":"armed","stem":"a-01","error":{"kind":"limitReached","message":"recording stopped at the length limit"}}"#
        );
        let plain = StateEvent::from(TakeEvent {
            state: RecState::Idle,
            stem: None,
            error: None,
        });
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"state":"idle"}"#
        );
        let capped = StateEvent::from(TakeEvent {
            state: RecState::Armed,
            stem: Some("a-01".into()),
            error: Some(TakeError::LimitReached),
        });
        assert_eq!(capped, full);
        assert_eq!(
            serde_json::to_string(&LevelEvent {
                peak: 0.5,
                rms: 0.25
            })
            .unwrap(),
            r#"{"peak":0.5,"rms":0.25}"#
        );
        assert_eq!(
            serde_json::to_string(&LevelEvent::from(Level {
                peak: 0.5,
                rms: 0.25
            }))
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

    // --- the app's wiring ---

    #[test]
    fn a_stop_writes_the_app_version() {
        let rig = rig();
        rig.take();
        rig.takes.stop(None).unwrap();
        let sidecar = sidecar_of(&rig, "main-01");
        assert_eq!(sidecar.app.version, env!("CARGO_PKG_VERSION"));
        assert_ne!(
            sidecar.app.version, "0.1.0",
            "not the record crate's version"
        );
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
    fn opening_a_project_repairs_it_in_the_background() {
        let rig = rig();
        let takes = Arc::clone(&rig.takes);
        rig.projects
            .on_open(move |project| repair_in_background(&takes, project));
        let other = Project::create(&rig.tmp.path().join("beta"), "beta").unwrap();
        let old = stale_partial(&other, "memo-01");
        let dir = other.root().to_string_lossy().into_owned();
        crate::project::open_project(&rig.projects, &AppData::at(rig.tmp.path()), &dir, |_| {
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
    fn list_and_stop_return_the_project_entry_camel_case() {
        let rig = rig();
        rig.take();
        let stopped = rig.takes.stop(None).unwrap();
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

    #[test]
    fn library_commands_reject_a_stale_root() {
        let rig = rig();
        rig.take();
        rig.takes.stop(None).unwrap();
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
        rig.takes.stop(None).unwrap();
        rig.start(&handle, Some("main")).unwrap();
        rig.takes.stop(None).unwrap();
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
