//! strudeland's audio engine: renders an [`EngineJob`] (the haps of one chunk, planned by the
//! render page) to float channel buffers as superdough does in Chromium's offline context.
//! Deterministic and single-threaded; the library does no I/O and never panics on job input.

pub(crate) mod adsr;
pub(crate) mod biquad;
pub(crate) mod bus;
pub(crate) mod chain;
pub(crate) mod compressor;
pub mod controls;
pub(crate) mod delay;
pub(crate) mod djf;
pub(crate) mod envelope;
pub(crate) mod fft;
pub(crate) mod job;
pub(crate) mod js;
pub mod jsmath;
pub(crate) mod ladder;
pub(crate) mod lfo;
pub mod live;
pub(crate) mod modulate;
pub(crate) mod noise;
pub mod orbit;
pub(crate) mod osc;
pub(crate) mod panner;
pub(crate) mod param;
pub(crate) mod periodic;
pub(crate) mod pulse;
pub(crate) mod render;
pub(crate) mod reverb;
pub(crate) mod shaper;
pub(crate) mod supersaw;
#[cfg(test)]
mod test_vectors;
pub(crate) mod transient;
pub(crate) mod voice;
pub(crate) mod vowel;
pub mod wav;
pub(crate) mod worklet;

pub use controls::{ALLOWED_KEYS, ALLOWED_SOUNDS, controls_json};
pub use job::{EngineJob, ErrorKind, JobError, OrbitSpec, RenderError, Value, ValueMap};
/// superdough's `mulberry32`, reused by the render pipeline's dither.
pub use js::Mulberry32;
/// JS `String(number)`, exposed for the headless host's `String(number)` differential test.
#[doc(hidden)]
pub use js::number_to_string;
pub use render::Engine;
pub use shaper::{DISTORTION_NAMES, distortion_algorithm};

/// The only sample rate the engine renders at.
pub const SAMPLE_RATE: u32 = 44100;
/// Frames per render quantum.
pub const QUANTUM: usize = 128;
/// [`SAMPLE_RATE`] as `f64`.
pub const SAMPLE_RATE_F64: f64 = SAMPLE_RATE as f64;
/// [`QUANTUM`] as `u64`, for frame arithmetic.
pub const QUANTUM_U64: u64 = QUANTUM as u64;

/// The time in seconds of the end of the quantum starting at frame `quantum_start`: an
/// `AudioHandler`'s `last_non_silent_time_` after a non-silent quantum.
pub fn quantum_end_time(quantum_start: u64) -> f64 {
    (quantum_start + QUANTUM_U64) as f64 / SAMPLE_RATE_F64
}

/// A rendered job: `channels.count` channels of exactly `frames` samples, and the render errors.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub channels: Vec<Vec<f32>>,
    pub errors: Vec<RenderError>,
}

/// Validates and renders a job.
pub fn render(job: &EngineJob) -> Result<Rendered, JobError> {
    Ok(Engine::new(job)?.run())
}
