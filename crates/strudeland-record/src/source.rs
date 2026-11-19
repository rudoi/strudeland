//! Where the capture's audio comes from: the [`Source`] seam, the cpal source and the fake.

use std::sync::atomic::Ordering;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, InputCallbackInfo, SampleFormat, SizedSample};

use crate::error::RecordError;
use crate::pipeline::{Capture, Shared};
use crate::recorder::{ArmedInfo, Msg};

/// The largest callback block the cpal source converts; a larger one is dropped and counted.
pub const MAX_BLOCK_FRAMES: usize = 16_384;

/// Which cpal timestamp an anchor's `host_ns` uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTime {
    /// cpal's `capture` instant.
    Capture,
    /// cpal's `callback` instant (`mHostTime` taken as the buffer's first frame).
    Callback,
    /// `callback` minus the device latency and safety offset, with the buffer counted once.
    CallbackMinusExtra,
}

/// The capture time the cpal source uses.
pub const DEFAULT_CAPTURE_TIME: CaptureTime = CaptureTime::Capture;

impl CaptureTime {
    /// The host time of a block's first frame from cpal's two instants (nanoseconds).
    pub fn host_ns(
        self,
        callback_ns: u64,
        capture_ns: u64,
        frames: usize,
        sample_rate: u32,
    ) -> u64 {
        match self {
            CaptureTime::Capture => capture_ns,
            CaptureTime::Callback => callback_ns,
            CaptureTime::CallbackMinusExtra => {
                let buffer_ns = (frames as u128 * 1_000_000_000 / u128::from(sample_rate.max(1)))
                    .min(u128::from(u64::MAX)) as u64;
                let extra = callback_ns
                    .saturating_sub(capture_ns)
                    .saturating_sub(buffer_ns);
                callback_ns.saturating_sub(extra).max(capture_ns)
            }
        }
    }
}

/// What kind of stream failure a fault is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    DeviceLost,
    Other,
}

/// A stream failure reported by a running source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub kind: FaultKind,
    pub message: String,
}

impl Fault {
    /// The fault a cpal error-callback error is; `None` for an `Xrun`, which is counted instead.
    pub fn from_cpal(error: &cpal::Error) -> Option<Fault> {
        use cpal::ErrorKind;
        let kind = match error.kind() {
            ErrorKind::Xrun => return None,
            ErrorKind::DeviceNotAvailable
            | ErrorKind::StreamInvalidated
            | ErrorKind::DeviceChanged => FaultKind::DeviceLost,
            _ => FaultKind::Other,
        };
        Some(Fault {
            kind,
            message: error.to_string(),
        })
    }
}

/// How a running source reports xruns and faults to the recorder. Cloneable and `Send`, so a
/// cpal error callback can own one; sending does not block.
#[derive(Clone)]
pub struct FaultSink {
    pub(crate) tx: Sender<Msg>,
    pub(crate) arm_id: u64,
    pub(crate) shared: Arc<Shared>,
}

impl FaultSink {
    pub fn xrun(&self) {
        self.shared.count_xrun();
    }

    pub fn fault(&self, fault: Fault) {
        let _ = self.tx.send(Msg::Fault {
            arm_id: self.arm_id,
            fault,
        });
    }

    /// Routes a cpal error-callback error: an xrun is counted, anything else is a fault.
    pub fn cpal_error(&self, error: &cpal::Error) {
        match Fault::from_cpal(error) {
            None => self.xrun(),
            Some(fault) => self.fault(fault),
        }
    }
}

/// A source of one channel of audio. `open` starts delivering blocks to
/// [`Capture::push_block`]; dropping the returned [`Running`] stops it.
pub trait Source {
    fn describe(&self) -> ArmedInfo;
    fn open(
        self: Box<Self>,
        capture: Capture,
        faults: FaultSink,
    ) -> Result<Box<dyn Running>, RecordError>;
}

/// A playing source; dropping it stops the stream.
pub trait Running {}

// --- cpal ---

/// An input device's stream, built and played at `open`.
pub struct CpalSource {
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
    info: ArmedInfo,
    capture_time: CaptureTime,
}

impl CpalSource {
    /// A source for `device` with its default input config, after the format and channel
    /// checks.
    pub fn new(
        device: cpal::Device,
        channel: u16,
        capture_time: CaptureTime,
    ) -> Result<CpalSource, RecordError> {
        let config = device
            .default_input_config()
            .map_err(|e| RecordError::from_cpal(&e))?;
        check_format(config.sample_format(), channel, config.channels())?;
        let id = device
            .id()
            .map_err(|e| RecordError::from_cpal(&e))?
            .to_string();
        let description = device
            .description()
            .map_err(|e| RecordError::from_cpal(&e))?;
        let info = ArmedInfo {
            device_id: id,
            device_name: description.name().to_string(),
            channel,
            channels: config.channels(),
            sample_rate: config.sample_rate(),
            bluetooth: description.interface_type() == cpal::InterfaceType::Bluetooth,
        };
        Ok(CpalSource {
            device,
            config,
            info,
            capture_time,
        })
    }
}

/// The supported sample formats, and the channel inside the device's count.
pub(crate) fn check_format(
    format: SampleFormat,
    channel: u16,
    channels: u16,
) -> Result<(), RecordError> {
    if !matches!(
        format,
        SampleFormat::F32 | SampleFormat::I16 | SampleFormat::I32
    ) {
        return Err(RecordError::UnsupportedFormat(format!(
            "sample format {format} is not f32, i16 or i32"
        )));
    }
    if channel >= channels {
        return Err(RecordError::UnsupportedFormat(format!(
            "channel {channel} is out of range for {channels} channels"
        )));
    }
    Ok(())
}

struct CpalRunning {
    _stream: cpal::Stream,
}

impl Running for CpalRunning {}

impl Source for CpalSource {
    fn describe(&self) -> ArmedInfo {
        self.info.clone()
    }

    fn open(
        self: Box<Self>,
        capture: Capture,
        faults: FaultSink,
    ) -> Result<Box<dyn Running>, RecordError> {
        let format = self.config.sample_format();
        let stream = match format {
            SampleFormat::F32 => self.build::<f32>(capture, faults)?,
            SampleFormat::I16 => self.build::<i16>(capture, faults)?,
            SampleFormat::I32 => self.build::<i32>(capture, faults)?,
            other => {
                return Err(RecordError::UnsupportedFormat(format!(
                    "sample format {other} is not f32, i16 or i32"
                )));
            }
        };
        stream.play().map_err(|e| RecordError::from_cpal(&e))?;
        Ok(Box::new(CpalRunning { _stream: stream }))
    }
}

impl CpalSource {
    fn build<T>(&self, mut capture: Capture, faults: FaultSink) -> Result<cpal::Stream, RecordError>
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let channels = usize::from(self.info.channels.max(1));
        let channel = usize::from(self.info.channel);
        let rate = self.info.sample_rate;
        let mode = self.capture_time;
        // Allocated once here; the callback only writes into it.
        let mut buf = vec![0.0f32; MAX_BLOCK_FRAMES];
        let data = move |data: &[T], info: &InputCallbackInfo| {
            let n = data.len() / channels;
            if n > MAX_BLOCK_FRAMES {
                capture.count_overflow();
                return;
            }
            for (i, out) in buf[..n].iter_mut().enumerate() {
                *out = f32::from_sample_(data[i * channels + channel]);
            }
            let ts = info.timestamp();
            let callback_ns = ts.callback.as_nanos().min(u128::from(u64::MAX)) as u64;
            let capture_ns = ts.capture.as_nanos().min(u128::from(u64::MAX)) as u64;
            capture.push_block(&buf[..n], mode.host_ns(callback_ns, capture_ns, n, rate));
        };
        let error = move |e: cpal::Error| faults.cpal_error(&e);
        self.device
            .build_input_stream::<T, _, _>(self.config.config(), data, error, None)
            .map_err(|e| RecordError::from_cpal(&e))
    }
}

// --- fake ---

type Slot = Arc<Mutex<Option<(Capture, FaultSink)>>>;

/// A source driven by hand from a test: [`FakeHandle::push`] delivers a block on the caller's
/// thread.
pub struct FakeSource {
    info: ArmedInfo,
    slot: Slot,
}

/// The test's end of a [`FakeSource`].
#[derive(Clone)]
pub struct FakeHandle {
    slot: Slot,
}

impl FakeSource {
    pub fn new(info: ArmedInfo) -> (FakeSource, FakeHandle) {
        let slot: Slot = Arc::new(Mutex::new(None));
        (
            FakeSource {
                info,
                slot: slot.clone(),
            },
            FakeHandle { slot },
        )
    }
}

struct FakeRunning {
    slot: Slot,
}

impl Running for FakeRunning {}

impl Drop for FakeRunning {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = None;
        }
    }
}

impl Source for FakeSource {
    fn describe(&self) -> ArmedInfo {
        self.info.clone()
    }

    fn open(
        self: Box<Self>,
        capture: Capture,
        faults: FaultSink,
    ) -> Result<Box<dyn Running>, RecordError> {
        *self.slot.lock().expect("fake slot") = Some((capture, faults));
        Ok(Box::new(FakeRunning { slot: self.slot }))
    }
}

impl FakeHandle {
    fn with<R>(&self, f: impl FnOnce(&mut Capture, &FaultSink) -> R) -> Option<R> {
        let mut slot = self.slot.lock().expect("fake slot");
        slot.as_mut().map(|(capture, sink)| f(capture, sink))
    }

    /// Delivers one block, as a callback would. False when the source is not open.
    pub fn push(&self, frames: &[f32], host_ns: u64) -> bool {
        self.with(|capture, _| capture.push_block(frames, host_ns))
            .is_some()
    }

    /// Reports a fault, as an error callback would. False when the source is not open.
    pub fn fault(&self, kind: FaultKind, message: &str) -> bool {
        self.with(|_, sink| {
            sink.fault(Fault {
                kind,
                message: message.to_string(),
            })
        })
        .is_some()
    }

    /// Counts a cpal xrun. False when the source is not open.
    pub fn xrun(&self) -> bool {
        self.with(|_, sink| sink.xrun()).is_some()
    }

    /// True while the source is open.
    pub fn is_open(&self) -> bool {
        self.with(|_, _| ()).is_some()
    }

    /// True while a take runs.
    pub fn is_recording(&self) -> bool {
        self.with(|capture, _| capture.shared().recording.load(Ordering::Acquire))
            .unwrap_or(false)
    }

    /// Waits until a take runs; false after `timeout`.
    pub fn wait_recording(&self, timeout: Duration) -> bool {
        wait_until(timeout, || self.is_recording())
    }

    /// Waits until the writer has drained the audio ring; false after `timeout`.
    pub fn wait_drained(&self, timeout: Duration) -> bool {
        wait_until(timeout, || {
            self.with(|c, _| c.free_audio_slots() == c.audio_capacity())
                .unwrap_or(true)
        })
    }
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The value the capture-timestamp evidence records (provisional until the loopback check);
    /// change both together.
    const PINNED_CAPTURE_TIME: &str = "Capture";

    #[test]
    fn capture_time_pin() {
        assert_eq!(format!("{DEFAULT_CAPTURE_TIME:?}"), PINNED_CAPTURE_TIME);
    }

    #[test]
    fn capture_time_candidates() {
        // 480 frames at 48 kHz is a 10 ms buffer; callback is 15 ms after capture.
        let (cb, cap) = (1_015_000_000u64, 1_000_000_000u64);
        assert_eq!(CaptureTime::Capture.host_ns(cb, cap, 480, 48_000), cap);
        assert_eq!(CaptureTime::Callback.host_ns(cb, cap, 480, 48_000), cb);
        // extra = 15 - 10 = 5 ms; callback - extra = capture + buffer.
        assert_eq!(
            CaptureTime::CallbackMinusExtra.host_ns(cb, cap, 480, 48_000),
            1_010_000_000
        );
    }

    #[test]
    fn callback_minus_extra_clamps() {
        // A buffer longer than callback - capture: no extra, the callback itself.
        assert_eq!(
            CaptureTime::CallbackMinusExtra.host_ns(1_005, 1_000, 480, 48_000),
            1_005
        );
        // capture after callback (cpal saturated): never before capture.
        assert_eq!(
            CaptureTime::CallbackMinusExtra.host_ns(0, 50, 480, 48_000),
            50
        );
        // A zero rate does not divide by zero.
        assert_eq!(CaptureTime::CallbackMinusExtra.host_ns(10, 5, 1, 0), 10);
    }

    #[test]
    fn faults_from_cpal_errors() {
        use cpal::ErrorKind;
        let f = |k: ErrorKind| Fault::from_cpal(&cpal::Error::from(k));
        assert_eq!(f(ErrorKind::Xrun), None);
        for kind in [
            ErrorKind::DeviceNotAvailable,
            ErrorKind::StreamInvalidated,
            ErrorKind::DeviceChanged,
        ] {
            assert_eq!(
                f(kind).map(|x| x.kind),
                Some(FaultKind::DeviceLost),
                "{kind:?}"
            );
        }
        assert_eq!(
            f(ErrorKind::BackendError).map(|x| x.kind),
            Some(FaultKind::Other)
        );
        assert_eq!(
            f(ErrorKind::Other).map(|x| x.message),
            Some(ErrorKind::Other.to_string())
        );
    }

    #[test]
    fn format_checks() {
        assert!(check_format(SampleFormat::F32, 0, 1).is_ok());
        assert!(check_format(SampleFormat::I16, 1, 2).is_ok());
        assert!(check_format(SampleFormat::I32, 7, 8).is_ok());
        let RecordError::UnsupportedFormat(m) = check_format(SampleFormat::U8, 0, 1).unwrap_err()
        else {
            panic!("not UnsupportedFormat");
        };
        assert_eq!(m, "sample format u8 is not f32, i16 or i32");
        let RecordError::UnsupportedFormat(m) = check_format(SampleFormat::F32, 2, 2).unwrap_err()
        else {
            panic!("not UnsupportedFormat");
        };
        assert_eq!(m, "channel 2 is out of range for 2 channels");
        assert!(matches!(
            check_format(SampleFormat::F64, 0, 2),
            Err(RecordError::UnsupportedFormat(_))
        ));
    }

    #[test]
    fn a_closed_fake_handle_reports_false() {
        let info = ArmedInfo {
            device_id: "fake:x".into(),
            device_name: "x".into(),
            channel: 0,
            channels: 1,
            sample_rate: 100,
            bluetooth: false,
        };
        let (_source, handle) = FakeSource::new(info);
        assert!(!handle.push(&[0.0], 0));
        assert!(!handle.fault(FaultKind::Other, "x"));
        assert!(!handle.xrun());
        assert!(!handle.is_open());
        assert!(!handle.is_recording());
        assert!(handle.wait_drained(Duration::ZERO));
        assert!(!handle.wait_recording(Duration::ZERO));
    }
}
