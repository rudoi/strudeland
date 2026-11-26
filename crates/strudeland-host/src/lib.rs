//! strudeland's headless host: evaluates a Strudel song in an embedded V8 and plans engine jobs
//! (schema 2) byte for byte as the render page does in Chromium, without a browser.
//!
//! Each request runs in a fresh V8 isolate and context on the [`Host`]'s worker thread: the natives
//! (`jsmath` `Math` overrides, a throwing `Math.random`, zero clocks, inert timers, console
//! capture) are installed, the committed bundle (`js/host.js`, [`bundle::BUNDLE`]) is
//! evaluated, its entry is called and the microtasks are drained; timers never fire.

#![deny(unsafe_code)]

pub mod bench;
pub mod bundle;
#[allow(unsafe_code)]
mod engine;
mod error;
mod host;
mod inspection;
pub mod math;
#[allow(unsafe_code)]
pub mod natives;
mod planned;
mod request;

pub use engine::v8_version;
pub use error::HostError;
pub use host::Host;
pub use inspection::{
    InspectedNote, InspectionRequest, InspectionResult, InspectionView, SourceSpan,
};
pub use planned::{
    ChunkInfo, ChunkWindow, LoopRange, Meter, MeterChange, PlanError, PlannedChunk, PlannedExport,
    PlannedRender, PlannedRepeat, PlannedWindow, Section, Structure,
};
pub use request::{
    ExportPreset, Layout, LengthRequest, LookbackSec, MAX_BARS, MAX_TIMEOUT_SEC, Mode,
    RenderPlanRequest, SAMPLE_RATE, SingleChunk, timeout_from_secs,
};
