//! What a render reports: progress events, the joined audio and the summary (the `--json` object
//! minus `path`).

use std::path::PathBuf;

use serde::Serialize;
use strudeland_host::{ChunkWindow, PlanError, PlannedWindow, Structure};

use crate::Format;
use crate::level::LoudnessSummary;

/// A step of a render, reported to the caller's callback from the calling thread.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// Planning finished: `chunks` jobs will be rendered.
    Planned { chunks: usize },
    /// Chunk `index` (0-based) of `of` finished rendering. Emitted from the calling thread, in
    /// completion order, exactly `of` times on success.
    ChunkRendered { index: usize, of: usize },
    /// An encode pass `pass` (1-based) finished (1 for a lossless format; 1 to 3 for lossy).
    Encoded { pass: u32 },
    /// The file was written.
    Written { path: PathBuf },
}

/// Where the time went, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timings {
    pub plan_ms: f64,
    pub render_ms: f64,
    pub level_ms: f64,
    pub encode_ms: f64,
    pub total_ms: f64,
}

/// A render's summary. Non-finite floats serialise as `null`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// The encoded format; `None` until [`finish`](crate::finish).
    pub format: Option<Format>,
    /// The encoded size; `None` until [`finish`](crate::finish).
    pub bytes: Option<u64>,
    /// The authored window: `[fromBar, toBar)` 0-based layout bars and the layout.
    pub window: PlannedWindow,
    /// Physical frames at `sample_rate`, including any appended repeat or decay.
    pub frames: u64,
    /// `frames / sample_rate`.
    pub seconds: f64,
    pub sample_rate: u32,
    /// Chunk jobs rendered.
    pub chunks: usize,
    /// Worker threads used (after clamping to the chunk count).
    pub jobs: usize,
    /// Loudness and true peak before and after leveling, the decoded peak of a lossy file and the
    /// encode passes; `None` until [`finish`](crate::finish).
    pub loudness: Option<LoudnessSummary>,
    pub timings: Timings,
    /// Planner and engine warnings (deduplicated and capped), in that order.
    pub warnings: Vec<String>,
}

/// The joined, unleveled audio of a render ([`render_audio`](crate::render_audio)).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedAudio {
    /// The joined window (`summary.window`): 2 channels of `summary.frames` samples at
    /// `summary.sample_rate`.
    pub channels: Vec<Vec<f32>>,
    /// The content window, before any export tail.
    pub music_frames: u64,
    pub cps: f64,
    pub structure: Structure,
    /// The full schedule's `'auto'` lookback, in seconds.
    pub lookback_sec: f64,
    pub chunks: Vec<ChunkWindow>,
    /// The schedule's events; only with `include_events`.
    pub events: Option<serde_json::Value>,
    /// The planner's captured and selection errors (also warnings in the summary).
    pub plan_errors: Vec<PlanError>,
    pub summary: Summary,
}

/// A finished render: the encoded file's bytes and the summary.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOutput {
    pub bytes: Vec<u8>,
    pub summary: Summary,
}

#[cfg(test)]
mod tests {
    use super::*;
    use strudeland_host::Layout;

    #[test]
    fn summary_keys_are_camel_case_and_non_finite_is_null() {
        let s = Summary {
            format: None,
            bytes: None,
            window: PlannedWindow {
                from_bar: 0,
                to_bar: 4,
                layout: Layout::Linear,
            },
            frames: 352_800,
            seconds: f64::NAN,
            sample_rate: 44_100,
            chunks: 1,
            jobs: 1,
            loudness: None,
            timings: Timings::default(),
            warnings: vec!["w".into()],
        };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"format":null,"bytes":null,"window":{"fromBar":0,"toBar":4,"layout":"linear"},"frames":352800,"seconds":null,"sampleRate":44100,"chunks":1,"jobs":1,"loudness":null,"timings":{"planMs":0.0,"renderMs":0.0,"levelMs":0.0,"encodeMs":0.0,"totalMs":0.0},"warnings":["w"]}"#
        );
    }
}
