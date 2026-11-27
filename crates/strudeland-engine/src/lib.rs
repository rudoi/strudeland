//! strudeland's audio engine: renders an [`EngineJob`] (the haps of one chunk, planned by the
//! render page) to float channel buffers as superdough does in Chromium's offline context.
//! Deterministic and single-threaded; the library does no I/O and never panics on job input.

pub(crate) mod adsr;
pub(crate) mod biquad;
pub(crate) mod bus;
pub mod capability;
pub(crate) mod chain;
pub(crate) mod coerce;
pub(crate) mod compressor;
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
pub(crate) mod params;
pub(crate) mod periodic;
pub(crate) mod pulse;
pub(crate) mod render;
pub(crate) mod render_core;
pub(crate) mod reverb;
pub(crate) mod shaper;
mod stems;
pub(crate) mod supersaw;
#[cfg(test)]
mod test_vectors;
pub(crate) mod transient;
pub(crate) mod voice;
pub(crate) mod vowel;
pub mod wav;
pub(crate) mod worklet;

/// The engine constants the export post-roll bound mirrors (`exportFilterTailSec` in
/// `src/lib/render/schedule.ts` and `exportDecay` in `src/lib/render/lookback.ts`).
/// `strudeland-render`'s `tests/decay_bound.rs` checks the bound against them.
/// `FEEDBACK_EXTRA_FRAMES` is also checked by behaviour, not only by that parity: the delay
/// unit test `each_loop_through_the_feedback_adds_one_quantum` renders the repeats it predicts.
/// `COMPRESSOR_PRE_DELAY` is the compressor's pre-delay in seconds (264 frames at the engine
/// rate), which the bound adds to a compressed voice's own need.
pub mod decay {
    pub use crate::biquad::{MAX_TAIL_AMPLITUDE, MAX_TAIL_TIME};
    pub use crate::compressor::K_PRE_DELAY as COMPRESSOR_PRE_DELAY;
    pub use crate::delay::{FEEDBACK_EXTRA_FRAMES, MAX_DELAY_TIME, MAX_FEEDBACK};
    pub use crate::reverb::IR_DURATION_FACTOR;
    pub use crate::vowel::FORMANTS;
}

pub use capability::{ALLOWED_KEYS, allowed_sounds, controls_json};
pub use job::{EngineJob, ErrorKind, JobError, OrbitSpec, RenderError, Value, ValueMap};
/// superdough's `mulberry32`, reused by the render pipeline's dither.
pub use js::Mulberry32;
/// JS `String(number)`, exposed for the headless host's `String(number)` differential test.
#[doc(hidden)]
pub use js::number_to_string;
pub use render::Engine;
pub use shaper::{DISTORTION_NAMES, distortion_algorithm};
pub use stems::{StemGrouping, StemId, StemLayout, render_with_stems};

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

/// A rendered job: planar output channels of exactly `frames` samples, and the render errors.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub channels: Vec<Vec<f32>>,
    pub errors: Vec<RenderError>,
}

/// Validates and renders a job.
pub fn render(job: &EngineJob) -> Result<Rendered, JobError> {
    Ok(Engine::new(job)?.run())
}
