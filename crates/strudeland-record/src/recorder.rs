//! The [`Recorder`]: a control thread that owns the source and the writer, and the public API
//! over it.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::anchors::Anchor;
use crate::error::RecordError;
use crate::levels::{Level, SystemClock};
use crate::pipeline::{Capture, Shared};
use crate::source::{Fault, FaultKind, FaultSink, Running, Source};
use crate::writer::{TakeData, WriterHandle, WriterMsg, max_frames, spawn_writer};

/// How long `start` waits for the take's first anchor.
pub const START_TIMEOUT: Duration = Duration::from_secs(2);

/// What to arm: a device (`None` is the default input) and its 0-based channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArmConfig {
    pub device_id: Option<String>,
    pub channel: u16,
}

/// The armed input, as the sidecar's `input` records it. `channel` is 0-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmedInfo {
    pub device_id: String,
    pub device_name: String,
    pub channel: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub bluetooth: bool,
}

/// Why a take ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    /// `stop`, `disarm` or shutdown.
    User,
    /// The length cap ([`max_frames`]).
    Cap,
    DeviceLost(String),
    WriteError(String),
}

/// A finished take. `frames` is the number of frames in the file; `xruns` counts cpal xruns
/// plus blocks dropped by a full ring during the take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeResult {
    pub path: PathBuf,
    pub frames: u64,
    pub sample_rate: u32,
    pub channel: u16,
    pub anchors: Vec<Anchor>,
    pub xruns: u32,
    pub ended: Ended,
}

/// What the recorder reports on its event channel.
#[derive(Debug)]
pub enum Event {
    Armed(ArmedInfo),
    /// About 30 Hz while armed.
    Level(Level),
    /// The take's first anchor.
    TakeStarted(Anchor),
    /// Exactly once per take, after the file is finalized.
    TakeEnded(TakeResult),
    Disarmed,
    /// The stream of arm `arm_id` (as [`Recorder::arm_with_id`] returned it) died; the
    /// recorder is disarmed. A queued `Failed` can arrive after a newer arm: compare the id.
    Failed {
        arm_id: u64,
        error: RecordError,
    },
}

/// Builds the source for an arm.
pub type SourceFactory = Box<dyn FnMut(&ArmConfig) -> Result<Box<dyn Source>, RecordError> + Send>;

/// Test limits: a lower cap and a shorter start timeout.
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Overrides [`max_frames`] when set.
    pub max_frames: Option<u64>,
    pub start_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_frames: None,
            start_timeout: START_TIMEOUT,
        }
    }
}

type Reply<T> = SyncSender<Result<T, RecordError>>;

pub(crate) enum Command {
    Arm(ArmConfig, Reply<(u64, ArmedInfo)>),
    Start(PathBuf, Reply<Anchor>),
    Stop(Reply<TakeResult>),
    Disarm(SyncSender<()>),
    Shutdown,
}

/// Everything the control thread receives.
pub(crate) enum Msg {
    Cmd(Command),
    Fault {
        arm_id: u64,
        fault: Fault,
    },
    /// The writer ended take `take_id` by itself (cap, write error).
    WriterEnded {
        take_id: u64,
    },
}

/// Captures one input channel to WAV files. All methods block until the control thread has
/// acted. Dropping it finalizes a running take.
pub struct Recorder {
    tx: Sender<Msg>,
    join: Option<JoinHandle<()>>,
}

fn gone() -> RecordError {
    RecordError::Backend("the recorder stopped".to_string())
}

impl Recorder {
    /// A recorder over the system's input devices. Opens nothing until `arm`.
    pub fn new() -> (Recorder, Receiver<Event>) {
        Recorder::with_source_factory(Box::new(crate::device::cpal_source))
    }

    /// A recorder whose sources come from `factory` (tests arm a fake).
    pub fn with_source_factory(factory: SourceFactory) -> (Recorder, Receiver<Event>) {
        Recorder::with_limits(factory, Limits::default())
    }

    #[doc(hidden)]
    pub fn with_limits(factory: SourceFactory, limits: Limits) -> (Recorder, Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        let (events, events_rx) = mpsc::channel();
        let control_tx = tx.clone();
        // Built on the thread: the armed state holds the `!Send` stream.
        let join = std::thread::Builder::new()
            .name("record-control".to_string())
            .spawn(move || {
                let control = Control {
                    factory,
                    limits,
                    events,
                    tx: control_tx,
                    armed: None,
                    next_arm: 0,
                    next_take: 0,
                    stored: None,
                };
                control.run(rx)
            })
            .expect("spawn the record-control thread");
        (
            Recorder {
                tx,
                join: Some(join),
            },
            events_rx,
        )
    }

    fn call<T>(&self, cmd: impl FnOnce(Reply<T>) -> Command) -> Result<T, RecordError> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.tx.send(Msg::Cmd(cmd(tx))).map_err(|_| gone())?;
        rx.recv().map_err(|_| gone())?
    }

    /// Opens and plays the input stream. Arming again with the same config is a no-op; with
    /// another it disarms first. `AlreadyRecording` while a take runs.
    pub fn arm(&self, config: ArmConfig) -> Result<ArmedInfo, RecordError> {
        self.arm_with_id(config).map(|(_, info)| info)
    }

    /// [`Recorder::arm`], also returning the arm's id: a new id per opened stream (a no-op
    /// re-arm keeps it), the one its [`Event::Failed`] carries.
    pub fn arm_with_id(&self, config: ArmConfig) -> Result<(u64, ArmedInfo), RecordError> {
        self.call(|r| Command::Arm(config, r))
    }

    /// Creates `path` (never replacing a file) and starts a take; returns its first anchor, or
    /// `StartTimeout` (the file removed) when none arrives in 2 s.
    pub fn start(&self, path: &Path) -> Result<Anchor, RecordError> {
        let path = path.to_path_buf();
        self.call(|r| Command::Start(path, r))
    }

    /// Finalizes the take and returns it. A take that ended by itself (cap, device loss) is
    /// returned once. `NotArmed` when there is neither.
    pub fn stop(&self) -> Result<TakeResult, RecordError> {
        self.call(Command::Stop)
    }

    /// Stops a running take (its result goes out as `TakeEnded`), closes the stream and sends
    /// `Disarmed`.
    pub fn disarm(&self) {
        let (tx, rx) = mpsc::sync_channel(1);
        if self.tx.send(Msg::Cmd(Command::Disarm(tx))).is_ok() {
            let _ = rx.recv();
        }
    }

    /// Finalizes a running take, disarms and joins the threads.
    pub fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        if let Some(join) = self.join.take() {
            let _ = self.tx.send(Msg::Cmd(Command::Shutdown));
            let _ = join.join();
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

struct Take {
    id: u64,
    path: PathBuf,
    suspect_at_start: u32,
}

struct Armed {
    config: ArmConfig,
    info: ArmedInfo,
    arm_id: u64,
    shared: Arc<Shared>,
    // Dropped before the writer quits (see `teardown`).
    running: Box<dyn Running>,
    writer: WriterHandle,
    take: Option<Take>,
}

struct Control {
    factory: SourceFactory,
    limits: Limits,
    events: Sender<Event>,
    tx: Sender<Msg>,
    armed: Option<Armed>,
    next_arm: u64,
    next_take: u64,
    /// The result of a take that ended by itself, for the next `stop`.
    stored: Option<TakeResult>,
}

impl Control {
    fn run(mut self, rx: Receiver<Msg>) {
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Cmd(Command::Arm(config, reply)) => {
                    let _ = reply.send(self.arm(config));
                }
                Msg::Cmd(Command::Start(path, reply)) => {
                    let _ = reply.send(self.start(path));
                }
                Msg::Cmd(Command::Stop(reply)) => {
                    let _ = reply.send(self.stop());
                }
                Msg::Cmd(Command::Disarm(reply)) => {
                    self.disarm();
                    let _ = reply.send(());
                }
                Msg::Cmd(Command::Shutdown) => {
                    self.disarm();
                    return;
                }
                Msg::Fault { arm_id, fault } => self.fault(arm_id, fault),
                Msg::WriterEnded { take_id } => self.writer_ended(take_id),
            }
        }
    }

    fn send(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn arm(&mut self, config: ArmConfig) -> Result<(u64, ArmedInfo), RecordError> {
        if let Some(armed) = &self.armed {
            if armed.take.is_some() {
                return Err(RecordError::AlreadyRecording);
            }
            if armed.config == config {
                return Ok((armed.arm_id, armed.info.clone()));
            }
            self.disarm();
        }
        let source = (self.factory)(&config)?;
        let info = source.describe();
        let shared = Arc::new(Shared::default());
        let (capture, rings) = Capture::new(info.sample_rate, shared.clone());
        let writer = spawn_writer(
            rings,
            self.events.clone(),
            self.tx.clone(),
            Box::new(SystemClock::new()),
        )?;
        self.next_arm += 1;
        let arm_id = self.next_arm;
        let sink = FaultSink {
            tx: self.tx.clone(),
            arm_id,
            shared: shared.clone(),
        };
        let running = match source.open(capture, sink) {
            Ok(running) => running,
            Err(e) => {
                writer.quit();
                return Err(e);
            }
        };
        self.armed = Some(Armed {
            config,
            info: info.clone(),
            arm_id,
            shared,
            running,
            writer,
            take: None,
        });
        self.send(Event::Armed(info.clone()));
        Ok((arm_id, info))
    }

    fn start(&mut self, path: PathBuf) -> Result<Anchor, RecordError> {
        let Some(armed) = &mut self.armed else {
            return Err(RecordError::NotArmed);
        };
        if armed.take.is_some() {
            return Err(RecordError::AlreadyRecording);
        }
        let file = File::create_new(&path)?;
        self.stored = None;
        self.next_take += 1;
        let id = self.next_take;
        let sample_rate = armed.info.sample_rate;
        let cap = self.limits.max_frames.unwrap_or(max_frames(sample_rate));
        let (first_tx, first_rx) = mpsc::sync_channel(1);
        let opened = armed.writer.call(|reply| WriterMsg::Open {
            id,
            file,
            path: path.clone(),
            sample_rate,
            max_frames: cap,
            first_anchor: first_tx,
            reply,
        });
        match opened {
            Some(Ok(())) => {}
            other => {
                let _ = std::fs::remove_file(&path);
                return Err(other.and_then(Result::err).unwrap_or_else(gone));
            }
        }
        let suspect_at_start = armed.shared.suspect_blocks();
        armed.shared.take_gen.fetch_add(1, Ordering::AcqRel);
        armed.shared.recording.store(true, Ordering::Release);
        match first_rx.recv_timeout(self.limits.start_timeout) {
            Ok(anchor) => {
                armed.take = Some(Take {
                    id,
                    path,
                    suspect_at_start,
                });
                let _ = self.events.send(Event::TakeStarted(anchor));
                Ok(anchor)
            }
            Err(_) => {
                armed.shared.recording.store(false, Ordering::Release);
                // The writer removes the file it was given.
                if armed.writer.call(WriterMsg::Abort).is_none() {
                    let _ = std::fs::remove_file(&path);
                }
                Err(RecordError::StartTimeout)
            }
        }
    }

    /// Ends the running take; `lost` replaces a user end with a device loss.
    fn finish_take(&mut self, lost: Option<String>) -> Option<TakeResult> {
        let armed = self.armed.as_mut()?;
        let take = armed.take.take()?;
        armed.shared.recording.store(false, Ordering::Release);
        let data = armed
            .writer
            .call(WriterMsg::Close)
            .flatten()
            .unwrap_or_else(|| TakeData {
                frames: 0,
                anchors: Vec::new(),
                ended: Ended::WriteError("the writer stopped".to_string()),
            });
        let ended = match (data.ended, lost) {
            (Ended::User, Some(message)) => Ended::DeviceLost(message),
            (ended, _) => ended,
        };
        Some(TakeResult {
            path: take.path,
            frames: data.frames,
            sample_rate: armed.info.sample_rate,
            channel: armed.info.channel,
            anchors: data.anchors,
            xruns: armed
                .shared
                .suspect_blocks()
                .saturating_sub(take.suspect_at_start),
            ended,
        })
    }

    fn stop(&mut self) -> Result<TakeResult, RecordError> {
        if let Some(result) = self.finish_take(None) {
            self.send(Event::TakeEnded(result.clone()));
            return Ok(result);
        }
        self.stored.take().ok_or(RecordError::NotArmed)
    }

    fn writer_ended(&mut self, take_id: u64) {
        let current = self
            .armed
            .as_ref()
            .and_then(|a| a.take.as_ref())
            .is_some_and(|t| t.id == take_id);
        if !current {
            return;
        }
        if let Some(result) = self.finish_take(None) {
            self.send(Event::TakeEnded(result.clone()));
            self.stored = Some(result);
        }
    }

    fn fault(&mut self, arm_id: u64, fault: Fault) {
        if self.armed.as_ref().is_none_or(|a| a.arm_id != arm_id) {
            return;
        }
        if let Some(result) = self.finish_take(Some(fault.message.clone())) {
            self.send(Event::TakeEnded(result.clone()));
            self.stored = Some(result);
        }
        self.teardown();
        let error = match fault.kind {
            FaultKind::DeviceLost => RecordError::DeviceLost(fault.message),
            FaultKind::Other => RecordError::Backend(fault.message),
        };
        self.send(Event::Failed { arm_id, error });
    }

    fn disarm(&mut self) {
        if self.armed.is_none() {
            return;
        }
        if let Some(result) = self.finish_take(None) {
            self.send(Event::TakeEnded(result));
        }
        self.teardown();
        self.send(Event::Disarmed);
    }

    /// Stops the stream, then the writer.
    fn teardown(&mut self) {
        if let Some(armed) = self.armed.take() {
            drop(armed.running);
            armed.writer.quit();
        }
    }
}
