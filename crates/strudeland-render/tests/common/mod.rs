//! Shared helpers for the render crate's integration tests.
#![allow(dead_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use strudeland_host::{Layout, LengthRequest};
use strudeland_render::{
    BitDepth, EncoderOptions, Loudness, OutputOptions, Progress, RenderOptions,
};

/// The repository root.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A corpus song's path (`tests/render/corpus/<file>`).
pub fn corpus_path(file: &str) -> PathBuf {
    repo_root().join("tests/render/corpus").join(file)
}

/// A corpus song's source.
pub fn corpus_song(file: &str) -> String {
    let path = corpus_path(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Loudness off, a 32-bit float WAV.
pub fn float_wav() -> OutputOptions {
    OutputOptions {
        loudness: Loudness::Off,
        encoder: EncoderOptions::Wav {
            bit_depth: BitDepth::F32,
        },
    }
}

pub fn options(length: LengthRequest) -> RenderOptions {
    RenderOptions {
        length,
        instruments: Default::default(),
        jobs: None,
        plan_timeout: Some(Duration::from_secs(120)),
        include_events: false,
        output: float_wav(),
    }
}

pub fn window(from_bar: u32, to_bar: u32, layout: Layout) -> LengthRequest {
    LengthRequest::Window {
        from_bar,
        to_bar,
        layout,
    }
}

/// A progress callback that records every event.
pub struct Recorder(pub RefCell<Vec<Progress>>);

impl Recorder {
    pub fn new() -> Self {
        Recorder(RefCell::new(Vec::new()))
    }

    pub fn record(&self, p: Progress) {
        self.0.borrow_mut().push(p);
    }

    pub fn events(&self) -> Vec<Progress> {
        self.0.borrow().clone()
    }
}
