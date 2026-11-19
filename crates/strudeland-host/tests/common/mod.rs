//! Shared helpers for the host's integration tests.
#![allow(dead_code)]

use std::path::PathBuf;

use strudeland_host::{Layout, LookbackSec, Mode, RenderRequest};

/// The repository root.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A corpus song's source (`tests/render/corpus/<file>`).
pub fn corpus_song(file: &str) -> String {
    let path = repo_root().join("tests/render/corpus").join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A linear mix request for `[0, to_bar)` with no lookback.
pub fn request(to_bar: u32) -> RenderRequest {
    RenderRequest {
        layout: Layout::Linear,
        from_bar: 0,
        to_bar,
        overlap_bars: 0,
        lookback_sec: LookbackSec::Seconds(0.0),
        mode: Mode::Mix,
        sample_rate: 44_100,
    }
}

/// A short synth song every test can plan quickly.
pub const SONG: &str = "$: note(\"c e g\").s(\"sine\")\n";
