//! strudeland microphone capture: one input channel to a mono 32-bit float WAV at the device's
//! rate, with levels and clock anchors from frame index to mach host time.
//!
//! The [`Recorder`] owns a control thread (the source and all state transitions) and, while
//! armed, a writer thread (the rings, the WAV, levels). The audio callback only touches atomics
//! and preallocated lock-free rings. Audio comes through the [`Source`] seam: [`CpalSource`] in
//! the app, [`testing::FakeSource`] in tests. [`takes`] runs the take lifecycle over the
//! recorder: [`takes::Takes`], its state, its ports and the memo files it publishes.

mod anchors;
pub mod clock;
mod device;
mod error;
mod levels;
mod pipeline;
mod recorder;
mod source;
pub mod takes;
mod writer;

pub use anchors::{Anchor, frame_at};
pub use clock::host_now_ns;
pub use device::{DeviceInfo, find_input_device, list_input_devices};
pub use error::RecordError;
pub use levels::Level;
pub use pipeline::Capture;
pub use recorder::{
    ArmConfig, ArmedInfo, Ended, Event, Recorder, START_TIMEOUT, SourceFactory, TakeResult,
};
pub use source::{
    CaptureTime, CpalSource, DEFAULT_CAPTURE_TIME, Fault, FaultKind, FaultSink, MAX_BLOCK_FRAMES,
    Running, Source,
};
pub use writer::max_frames;

/// The fake source, test limits, the take lifecycle's fakes and the rig over them, for this
/// crate's tests and the app's. It is compiled into normal builds, as `FakeSource` always was,
/// because the app's tests use it across the crate boundary; nothing outside tests calls it.
#[doc(hidden)]
pub mod testing {
    pub use crate::recorder::Limits;
    pub use crate::source::{FakeHandle, FakeSource};
    pub use crate::takes::testing::{
        CollectingSink, FakePlatform, RATE, T0, TakesRig, WAIT, block_publication, fake_maker,
        sidecar_of, start_fields,
    };
}
