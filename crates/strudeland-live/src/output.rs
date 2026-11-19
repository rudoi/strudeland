//! The seam between the engine thread and the audio device.

/// Fills `buf` (interleaved stereo, `buf.len() == 2 * frames`) with device-rate frames. Runs on
/// the audio thread: it must not allocate, lock or block.
pub type RenderCallback = Box<dyn FnMut(&mut [f32], PlaybackTime) + Send + 'static>;

/// When a callback's buffer is heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaybackTime {
    /// Host time in nanoseconds of the buffer's first frame (mach host time on macOS).
    pub host_ns: u64,
}

/// What a started stream runs at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputInfo {
    pub sample_rate: u32,
    pub buffer_frames: Option<u32>,
}

/// A device event the engine thread polls for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputNotice {
    DeviceChanged,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputError {
    #[error("no audio output device")]
    NoDevice,
    #[error("unsupported output: {0}")]
    Unsupported(String),
    #[error("output stream: {0}")]
    Stream(String),
}

/// An audio output: [`CpalOutput`](crate::CpalOutput) in the app, the fake in tests.
pub trait Output: Send {
    /// Opens the device's stream and starts calling `render`; returns once it plays.
    fn start(&mut self, render: RenderCallback) -> Result<OutputInfo, OutputError>;
    /// Stops calling `render` and releases the device (idempotent).
    fn stop(&mut self);
    /// A device change or a stream failure since the last call (polled by the engine thread
    /// every loop pass, never from the audio callback).
    fn poll_notice(&mut self) -> Option<OutputNotice> {
        None
    }
}
