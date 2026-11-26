//! Shared helpers for the host's integration tests.
#![allow(dead_code)]

use std::path::PathBuf;

use strudeland_host::{
    Layout, LengthRequest, LookbackSec, Mode, PlannedRender, RenderPlanRequest, SingleChunk,
};

/// The repository root.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A corpus song's source (`tests/render/corpus/<file>`).
pub fn corpus_song(file: &str) -> String {
    let path = repo_root().join("tests/render/corpus").join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The single-chunk request for `[from_bar, to_bar)` in `layout`, with `chunk`'s overlap,
/// lookback and mode.
pub fn single_chunk(
    from_bar: u32,
    to_bar: u32,
    layout: Layout,
    chunk: SingleChunk,
) -> RenderPlanRequest {
    RenderPlanRequest {
        instruments: Default::default(),
        length: LengthRequest::Window {
            from_bar,
            to_bar,
            layout,
        },
        preset: None,
        include_events: None,
        chunk: Some(chunk),
        timeout_sec: None,
    }
}

/// A single-chunk linear mix request for `[0, to_bar)` with no lookback.
pub fn request(to_bar: u32) -> RenderPlanRequest {
    single_chunk(
        0,
        to_bar,
        Layout::Linear,
        SingleChunk {
            overlap_bars: 0,
            lookback_sec: LookbackSec::Seconds(0.0),
            mode: Mode::Mix,
        },
    )
}

/// The one chunk's job text of a single-chunk plan.
pub fn job(planned: &PlannedRender) -> &str {
    assert_eq!(planned.chunks.len(), 1, "not a single-chunk plan");
    &planned.chunks[0].json
}

/// A short synth song every test can plan quickly.
pub const SONG: &str = "$: note(\"c e g\").s(\"sine\")\n";
