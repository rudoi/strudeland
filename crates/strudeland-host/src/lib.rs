//! strudeland's headless host: evaluates a Strudel song in an embedded V8 and plans its
//! engine job (schema 2) byte for byte as the render page does in Chromium, without a browser.
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
pub mod math;
#[allow(unsafe_code)]
pub mod natives;
mod planned;
mod request;

pub use engine::v8_version;
pub use error::HostError;
pub use host::{Host, PlannedJob};
pub use planned::{
    ChunkWindow, Meter, PlanError, PlannedChunk, PlannedExport, PlannedRender, PlannedRepeat,
    PlannedWindow, Section, Structure,
};
pub use request::{
    ExportPreset, Layout, LengthRequest, LookbackSec, MAX_BARS, MAX_LOOPS, MAX_TIMEOUT_SEC, Mode,
    RenderPlanRequest, RenderRequest, SAMPLE_RATE, timeout_from_secs,
};
