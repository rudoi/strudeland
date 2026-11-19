//! Frame-level pitch tracking: the [`PitchTracker`] seam and its pYIN implementation.

mod pyin;

pub use pyin::PyinTracker;

/// Turns mono audio into a frame-level pitch track. Implementations are deterministic: the same
/// samples give a bitwise identical track.
pub trait PitchTracker {
    /// Tracks `signal` at `rate` Hz (the pipeline passes the analysis rate), finite samples as
    /// `util::finite_f64` gives them. Input shorter than one frame gives a track with no frames.
    fn track_signal(&self, signal: &[f64], rate: u32) -> PitchTrack;
}

/// The frames of a pitch track, one per hop; frame `i` is centred at `i * hop_seconds`.
#[derive(Debug, Clone, PartialEq)]
pub struct PitchTrack {
    /// Seconds between frames.
    pub hop_seconds: f32,
    pub frames: Vec<PitchFrame>,
}

/// One frame of a pitch track.
#[derive(Debug, Clone, PartialEq)]
pub struct PitchFrame {
    /// The fundamental in Hz, `None` when the frame is unvoiced.
    pub f0: Option<f32>,
    /// The probability that the frame is voiced, 0..=1.
    pub voiced_prob: f32,
    /// The frame's RMS level.
    pub rms: f32,
}
