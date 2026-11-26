//! The take lifecycle: arm, disarm, start, stop, the endings the recorder reports by itself
//! (cap, write error, device loss), reload, quit and the repair skip at project open.
//!
//! [`Takes`] owns the [`Recorder`] and its [`TakeState`] (idle, armed, recording or ended) over
//! three ports: [`Platform`] (the microphone permission and the input devices), [`Sink`] (the
//! state events and levels) and [`RecorderMaker`]. A take is one recording (`<stem>.wav` and its
//! sidecar); `Takes` is the type that runs them. The app (`src-tauri/src/recording.rs`) maps the
//! domain types and [`TakeError`] to its wire types and adapts the ports; tests use the fakes in
//! [`crate::testing`].
//!
//! A take is finalized into the project captured at start, even if another project was opened
//! meanwhile.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::thread::JoinHandle;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use strudeland_project::{
    HostNs, Project, RecordingEntry, SidecarBacking, SidecarClockOffsets, SidecarOutput,
    SidecarTempo, format_rfc3339_utc,
};

use crate::{
    ArmConfig, ArmedInfo, DeviceInfo, Ended, Event, Level, RecordError, Recorder, TakeResult,
};

mod error;
mod prefs;
mod sidecar;
pub(crate) mod testing;
#[cfg(test)]
mod tests;

pub use error::TakeError;
pub use prefs::{Prefs, write_prefs};

use prefs::{read_prefs, resolve_selection, saved_channel};
use sidecar::{TakeMeta, complete_ended, memo, publish_fallback};

// --- domain types ---

/// The microphone permission (AVFoundation's authorization status, or the OS's equivalent).
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

/// What [`Takes::status`] reports.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeStatus {
    pub state: RecState,
    pub permission: Permission,
    /// The armed device; `None` in `idle`.
    pub device: Option<DeviceInfo>,
    /// 0-based: the armed channel, or in `idle` the saved one (0 when none).
    pub channel: u16,
    /// Set only while `recording`.
    pub stem: Option<String>,
    pub elapsed_seconds: Option<f64>,
}

/// A device and its 0-based channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub device_id: String,
    pub channel: u16,
}

/// What [`Takes::devices`] lists.
#[derive(Debug, Clone, PartialEq)]
pub struct Devices {
    pub devices: Vec<DeviceInfo>,
    /// The saved choice resolved against the present devices; `None` without any input device.
    pub selected: Option<Selection>,
}

/// What a take starts with: enough for a timing-null sidecar if the webview never stops.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeStart {
    pub tempo: SidecarTempo,
    pub backing: SidecarBacking,
}

/// The webview's part of a stopped take's sidecar; the rest of the schema comes from the take.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeStop {
    pub output: SidecarOutput,
    pub tempo: SidecarTempo,
    pub backing: SidecarBacking,
    pub bar1_host_ns: Option<HostNs>,
    pub clock_offset_ns: SidecarClockOffsets,
    /// `[contextTime, hostNs]` pairs.
    pub output_stamps: Vec<(f64, HostNs)>,
}

/// A state transition the webview hears about.
#[derive(Debug, Clone, PartialEq)]
pub struct TakeEvent {
    pub state: RecState,
    pub stem: Option<String>,
    pub error: Option<TakeError>,
}

impl TakeEvent {
    fn new(state: RecState) -> TakeEvent {
        TakeEvent {
            state,
            stem: None,
            error: None,
        }
    }

    fn stem(mut self, stem: impl Into<String>) -> TakeEvent {
        self.stem = Some(stem.into());
        self
    }

    fn error(mut self, error: TakeError) -> TakeEvent {
        self.error = Some(error);
        self
    }
}

/// What the app passes in.
#[derive(Debug, Clone)]
pub struct TakesConfig {
    /// The sidecar's `app.version`: the app's version, not this crate's.
    pub app_version: String,
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
    fn state(&self, event: TakeEvent);
    fn level(&self, level: Level);
}

/// Builds the recorder at the first arm.
pub type RecorderMaker = Box<dyn Fn() -> (Recorder, Receiver<Event>) + Send + Sync>;

// --- state ---

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
    device: DeviceInfo,
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
    /// Idle after anything but a refused stream open.
    fn idle() -> TakeState {
        TakeState::Idle {
            stream_denied: false,
        }
    }

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
            state: TakeState::idle(),
            pending_loss_stem: None,
        }
    }
}

/// The recorder and every take it runs. One per app (one window), shared with the event
/// forwarder thread, which holds only a [`Weak`] to it.
///
/// The lock is held across arm (the recorder's arm), start (up to the start timeout) and stop;
/// levels and the permission request run outside it.
pub struct Takes {
    inner: Mutex<Inner>,
    requesting: AtomicBool,
    platform: Box<dyn Platform>,
    sink: Arc<dyn Sink>,
    maker: RecorderMaker,
    config: TakesConfig,
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
            None => TakeState::idle(),
        };
        leave(inner, next);
    }
}

/// Drains the recorder's events until the channel closes (at shutdown).
fn forward(takes: Weak<Takes>, events: Receiver<Event>) {
    for event in events {
        let Some(takes) = takes.upgrade() else {
            return;
        };
        takes.on_event(event);
    }
}

/// Clears `requesting` when the permission request ends, even by a panic.
struct Requesting<'a>(&'a AtomicBool);

impl Drop for Requesting<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Stops a running take and publishes it with the fallback sidecar, and publishes an ended
/// take not published yet (logging failures).
fn finalize_running(inner: &mut Inner) {
    settle_ended(inner);
    if !inner.state.is_recording() {
        return;
    }
    let TakeState::Recording { armed, take } =
        std::mem::replace(&mut inner.state, TakeState::idle())
    else {
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

impl Takes {
    pub fn new(
        platform: Box<dyn Platform>,
        sink: Arc<dyn Sink>,
        maker: RecorderMaker,
        config: TakesConfig,
    ) -> Arc<Takes> {
        Arc::new(Takes {
            inner: Mutex::new(Inner::default()),
            requesting: AtomicBool::new(false),
            platform,
            sink,
            maker,
            config,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock leaves the data consistent, so recover from poison.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn emit(&self, event: TakeEvent) {
        self.sink.state(event);
    }

    /// Creates the recorder and its event forwarder at the first arm.
    fn ensure_recorder<'a>(self: &Arc<Self>, inner: &'a mut Inner) -> &'a Recorder {
        if inner.recorder.is_none() {
            let (recorder, events) = (self.maker)();
            let weak = Arc::downgrade(self);
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

    /// One recorder event, as the forwarder thread delivers it: levels go to the sink without
    /// the lock, an ending the recorder reached by itself and a stream failure change the state,
    /// and every other event is ignored.
    pub fn on_event(&self, event: Event) {
        match event {
            Event::Level(level) => self.sink.level(level),
            Event::TakeEnded(result) if result.ended != Ended::User => self.ended_by_itself(result),
            Event::Failed { arm_id, error } => self.failed(arm_id, error),
            _ => {}
        }
    }

    /// A take the recorder ended (cap, device loss, write error): publish it with the fallback
    /// sidecar and keep it for the webview's stop. A take a command already took is ignored.
    fn ended_by_itself(&self, result: TakeResult) {
        let mut inner = self.lock();
        let (armed, take) = match std::mem::replace(&mut inner.state, TakeState::idle()) {
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
            Ended::Cap => self.emit(
                TakeEvent::new(RecState::Armed)
                    .stem(stem)
                    .error(TakeError::LimitReached),
            ),
            // The `Failed` event that follows reports it.
            Ended::DeviceLost(_) => inner.pending_loss_stem = Some(stem),
            Ended::WriteError(message) => self.emit(
                TakeEvent::new(RecState::Armed)
                    .stem(stem)
                    .error(TakeError::WriteFailed(message)),
            ),
            Ended::User => {}
        }
    }

    /// The stream of arm `arm_id` died: the recorder is disarmed. A `Failed` of an older arm,
    /// queued while the webview armed again, is ignored (the recorder already replaced it).
    fn failed(&self, arm_id: u64, error: RecordError) {
        let mut inner = self.lock();
        let stem = inner.pending_loss_stem.take();
        if inner.state.armed().is_some_and(|a| a.arm_id != arm_id) {
            return;
        }
        match &mut inner.state {
            TakeState::Armed(_) => inner.state = TakeState::idle(),
            TakeState::Ended { armed, .. } => *armed = None,
            // Recording is unreachable with a matching id (`TakeEnded` comes first); its take stays.
            TakeState::Idle { .. } | TakeState::Recording { .. } => {}
        }
        let mut event = TakeEvent::new(RecState::Idle).error(error.into());
        event.stem = stem;
        self.emit(event);
    }

    pub fn status(&self, prefs_path: &Path) -> TakeStatus {
        let permission = self.platform.permission();
        let inner = self.lock();
        let armed = inner.state.armed();
        let permission =
            if permission == Permission::NotDetermined && !self.platform.has_permission_prompt() {
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
        TakeStatus {
            state,
            permission,
            device,
            channel,
            stem,
            elapsed_seconds,
        }
    }

    /// An exclusive input can refuse a second config query while our own stream is open.
    fn input_devices(&self) -> Result<Vec<DeviceInfo>, TakeError> {
        let mut list = self.platform.devices()?;
        if let Some(armed) = self.lock().state.armed()
            && let Some(device) = list
                .iter_mut()
                .find(|d| d.id == armed.info.device_id && d.channels == 0)
        {
            device.channels = armed.info.channels;
            device.sample_rate = armed.info.sample_rate;
        }
        Ok(list)
    }

    pub fn devices(&self, prefs_path: &Path) -> Result<Devices, TakeError> {
        let list = self.input_devices()?;
        let prefs = read_prefs(prefs_path);
        let selected = resolve_selection(&list, &prefs).map(|(d, channel)| Selection {
            device_id: d.id.clone(),
            channel,
        });
        Ok(Devices {
            devices: list,
            selected,
        })
    }

    /// The permission, asking first when it is not determined. `PermissionPending` while
    /// another arm asks.
    fn ensure_permission(&self) -> Result<(), TakeError> {
        let mut permission = self.platform.permission();
        if permission == Permission::NotDetermined {
            if !self.platform.has_permission_prompt() {
                // The actual device open will decide and report access.
                return Ok(());
            }
            if self.requesting.swap(true, Ordering::AcqRel) {
                return Err(TakeError::PermissionPending);
            }
            let _requesting = Requesting(&self.requesting);
            self.platform.request_permission();
            // The status, not the handler's answer, decides.
            permission = self.platform.permission();
        }
        let error = match permission {
            Permission::Authorized => return Ok(()),
            Permission::Restricted => TakeError::PermissionRestricted,
            Permission::NotDetermined | Permission::Denied => TakeError::PermissionDenied,
        };
        self.emit(TakeEvent::new(RecState::Idle).error(error.clone()));
        Err(error)
    }

    /// Opens the stream on the chosen input (the arguments, else the saved prefs, else the
    /// default input). Saves the prefs only when an argument was given.
    pub fn arm(
        self: &Arc<Self>,
        prefs_path: &Path,
        device_id: Option<String>,
        channel: Option<u16>,
    ) -> Result<TakeStatus, TakeError> {
        if self.requesting.load(Ordering::Acquire) {
            return Err(TakeError::PermissionPending);
        }
        if self.lock().state.is_recording() {
            return Err(TakeError::Busy);
        }
        self.ensure_permission()?;
        let channel_arg = channel;
        let list = self.input_devices()?;
        let prefs = read_prefs(prefs_path);
        let (device, channel) = match device_id.as_deref() {
            Some(id) => {
                let device = list
                    .iter()
                    .find(|d| d.id == id)
                    .ok_or_else(|| TakeError::from(RecordError::NoDevice))?;
                let channel = channel.unwrap_or(saved_channel(device, &prefs));
                (device, channel)
            }
            None => {
                let (device, saved) = resolve_selection(&list, &prefs)
                    .ok_or_else(|| TakeError::from(RecordError::NoDevice))?;
                (device, channel.unwrap_or(saved))
            }
        };
        if device.channels != 0 && channel >= device.channels {
            return Err(TakeError::ChannelOutOfRange {
                channel,
                channels: device.channels,
            });
        }
        {
            let mut inner = self.lock();
            if inner.state.is_recording() {
                return Err(TakeError::Busy);
            }
            settle_ended(&mut inner);
            let recorder = self.ensure_recorder(&mut inner);
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
                    inner.state = TakeState::idle();
                    return Err(error.into());
                }
            };
            inner.state = TakeState::Armed(ArmedInput {
                arm_id,
                device: DeviceInfo {
                    channels: info.channels,
                    sample_rate: info.sample_rate,
                    ..device.clone()
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
        Ok(self.armed_status(prefs_path))
    }

    fn armed_status(&self, prefs_path: &Path) -> TakeStatus {
        self.emit(TakeEvent::new(RecState::Armed));
        self.status(prefs_path)
    }

    /// Closes the stream. `Busy` while a take runs.
    pub fn disarm(&self, prefs_path: &Path) -> Result<TakeStatus, TakeError> {
        {
            let mut inner = self.lock();
            if inner.state.is_recording() {
                return Err(TakeError::Busy);
            }
            settle_ended(&mut inner);
            if let Some(recorder) = &inner.recorder {
                recorder.disarm();
            }
            inner.state = match inner.state {
                TakeState::Idle { stream_denied } => TakeState::Idle { stream_denied },
                _ => TakeState::idle(),
            };
        }
        self.emit(TakeEvent::new(RecState::Idle));
        Ok(self.status(prefs_path))
    }

    /// Starts a take named by Rust (`<songStem>-NN` or `memo-NN`) in the project `resolve`
    /// returns (called under the lock, after the busy and armed checks; its error comes back as
    /// it is); returns the stem once the writer has received samples.
    pub fn start<E: From<TakeError>>(
        &self,
        resolve: impl FnOnce() -> Result<Arc<Project>, E>,
        song_stem: Option<&str>,
        fields: TakeStart,
    ) -> Result<String, E> {
        let mut inner = self.lock();
        if inner.state.is_recording() {
            return Err(TakeError::Busy.into());
        }
        let (Some(armed), Some(recorder)) = (inner.state.armed(), &inner.recorder) else {
            return Err(TakeError::NotArmed.into());
        };
        let project = resolve()?;
        let stem = project.default_recording_stem(song_stem).map_err(memo)?;
        let path = project.recording_partial_path(&stem).map_err(memo)?;
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        recorder.start(&path).map_err(TakeError::from)?;
        let input = armed.info.clone();
        // An ended take is published first (if it is not yet), before the new take's clock
        // starts, and leaves its armed input behind.
        settle_ended(&mut inner);
        let TakeState::Armed(armed) = std::mem::replace(&mut inner.state, TakeState::idle()) else {
            unreachable!("the state had an armed input");
        };
        inner.state = TakeState::Recording {
            armed,
            take: Take {
                meta: TakeMeta {
                    project,
                    stem: stem.clone(),
                    start: fields,
                    input,
                    created_at: format_rfc3339_utc(created_at),
                    app_version: self.config.app_version.clone(),
                },
                path,
                started: Instant::now(),
            },
        };
        self.emit(TakeEvent::new(RecState::Recording).stem(stem.clone()));
        Ok(stem)
    }

    /// Stops the take and publishes it into the project it started in, with the webview's
    /// `fields` (`None`: the fallback sidecar). For a take Rust already finalized (cap, device
    /// loss) it replaces that take's sidecar instead. `NoTake` with nothing to stop.
    ///
    /// A take whose publish fails is kept as an unpublished ended take (the state event carries
    /// the error): another stop retries it, and arm, start, disarm, reload and quit publish it
    /// with the fallback sidecar.
    pub fn stop(&self, fields: Option<TakeStop>) -> Result<RecordingEntry, TakeError> {
        let mut inner = self.lock();
        if inner.state.is_recording() {
            let TakeState::Recording { armed, take } =
                std::mem::replace(&mut inner.state, TakeState::idle())
            else {
                unreachable!("the state is recording");
            };
            let Some(recorder) = &inner.recorder else {
                inner.state = TakeState::Armed(armed);
                return Err(TakeError::NotArmed);
            };
            let result = match recorder.stop() {
                Ok(result) => result,
                Err(e) => {
                    // The take is dropped; its partial is left to the next open's repair.
                    inner.state = TakeState::Armed(armed);
                    eprintln!("recording: stopping {} failed: {e}", take.meta.stem);
                    return Err(TakeError::from(e));
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
        let (armed, mut ended) = match std::mem::replace(&mut inner.state, TakeState::idle()) {
            TakeState::Ended { armed, ended } => (armed, ended),
            other => {
                inner.state = other;
                return Err(TakeError::NoTake);
            }
        };
        let state = if armed.is_some() {
            RecState::Armed
        } else {
            RecState::Idle
        };
        inner.state = armed.map(TakeState::Armed).unwrap_or_else(TakeState::idle);
        let stem = ended.meta.stem.clone();
        match complete_ended(&mut ended, fields.as_ref()) {
            Ok(entry) => {
                self.emit(TakeEvent::new(state).stem(stem));
                Ok(entry)
            }
            Err(e) => {
                if !ended.published {
                    eprintln!("recording: publishing {stem} failed: {e}");
                    let armed = match std::mem::replace(&mut inner.state, TakeState::idle()) {
                        TakeState::Armed(armed) => Some(armed),
                        _ => None,
                    };
                    inner.state = TakeState::Ended { armed, ended };
                }
                self.emit(TakeEvent::new(state).stem(stem).error(e.clone()));
                Err(e)
            }
        }
    }

    /// The webview is reloading: a take in progress is finalized with the fallback sidecar and
    /// the recorder is disarmed, without events (the new page asks for the status). Idle is a
    /// no-op.
    pub fn on_reload(&self) {
        let mut inner = self.lock();
        if matches!(inner.state, TakeState::Idle { .. }) {
            return;
        }
        finalize_running(&mut inner);
        if let Some(recorder) = &inner.recorder {
            recorder.disarm();
        }
        inner.state = TakeState::idle();
    }

    /// The app quits: finalizes a take in progress with the fallback sidecar, then shuts the
    /// recorder down and joins it and the event forwarder. Emits nothing.
    pub fn shutdown(&self) {
        let (recorder, forwarder) = {
            let mut inner = self.lock();
            finalize_running(&mut inner);
            if !matches!(inner.state, TakeState::Idle { .. }) {
                inner.state = TakeState::idle();
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

    /// Whether a take runs or just ended in the project at `root`.
    pub(crate) fn holds_project(&self, root: &Path) -> bool {
        let inner = self.lock();
        let here = |p: &Project| p.root() == root;
        match &inner.state {
            TakeState::Recording { take, .. } => here(&take.meta.project),
            TakeState::Ended { ended, .. } => here(&ended.meta.project),
            TakeState::Idle { .. } | TakeState::Armed(_) => false,
        }
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

/// Recovers the `.partial` files a crash left in `project`, unless a take of `takes` is in
/// progress or ended there.
pub fn repair_on_open(takes: &Takes, project: &Project) -> RepairOutcome {
    if takes.holds_project(project.root()) {
        return RepairOutcome::Skipped;
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

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn raw_permission_values() {
        assert_eq!(Permission::from_raw(0), Permission::NotDetermined);
        assert_eq!(Permission::from_raw(1), Permission::Restricted);
        assert_eq!(Permission::from_raw(2), Permission::Denied);
        assert_eq!(Permission::from_raw(3), Permission::Authorized);
        assert_eq!(Permission::from_raw(4), Permission::Denied);
        assert_eq!(Permission::from_raw(-1), Permission::Denied);
    }
}
