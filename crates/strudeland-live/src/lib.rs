//! strudeland live playback: batches of haps in, audio out through a ring to the device, with
//! clock anchors, stats, scope and level frames coming back as [`Event`]s.
//!
//! Audio goes through the [`Output`] seam: [`CpalOutput`] in the app, [`testing::FakeOutput`]
//! in tests, so nothing here needs hardware to be tested.

mod batch;
mod cpal_output;
mod error;
mod event;
mod fake;
mod ir_worker;
mod master;
mod output;
mod player;
mod resample;
mod ring;
mod rt;

pub use batch::{Batch, HapEntry, ValueEntry};
pub use cpal_output::CpalOutput;
pub use error::LiveError;
pub use event::{BatchDecodeUs, Event, Stats};
pub use master::LevelFrame;
pub use output::{Output, OutputError, OutputInfo, OutputNotice, PlaybackTime, RenderCallback};
pub use player::Player;
pub use rt::realtime_refusals;
pub use strudeland_engine::OrbitSpec;

/// The fake output, for this crate's tests and the app's, and the resampler (whose delay the
/// player tests account for).
#[doc(hidden)]
pub mod testing {
    pub use crate::fake::{FAKE_EPOCH_NS, FakeHandle, FakeOutput, PULLED_CAP_SAMPLES, Pace};
    pub use crate::resample::QuantumResampler;
}
