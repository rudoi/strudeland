//! Helpers shared by the integration tests (each test crate uses a subset).
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use strudeland_engine::EngineJob;

/// The repository root.
pub fn repo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// `tests/render/corpus.json`: the render corpus manifest.
pub fn corpus_path() -> PathBuf {
    repo_root().join("tests/render/corpus.json")
}

/// `tests/refs/`: the Chromium references and their jobs.
pub fn refs_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/refs"))
}

/// The parsed JSON file at `path`; panics with the path on any error.
pub fn read_json(path: &Path) -> Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The job `tests/refs/<file>`, or why it cannot be read or parsed.
pub fn try_load_job(file: &str) -> Result<EngineJob, String> {
    let text = std::fs::read_to_string(refs_dir().join(file))
        .map_err(|e| format!("cannot read {file}: {e}"))?;
    EngineJob::from_json(&text).map_err(|e| format!("{file}: {e}"))
}

/// The job `tests/refs/<file>`; panics on any error.
pub fn load_job(file: &str) -> EngineJob {
    try_load_job(file).unwrap_or_else(|e| panic!("{e}"))
}

/// One pause of [`base_job`] with its single hap (value 0).
#[derive(Debug, Clone, Copy)]
pub struct PauseHap {
    pub suspend_sec: f64,
    pub t: f64,
    pub duration: f64,
}

/// A schema-2 job of `frames` frames with `channels` output channels (multi-channel orbits above
/// 2) and orbit 1 on channels 0 and 1: two pauses, bar 0 and bar 1, each with one hap of value 0.
pub fn base_job(frames: u64, channels: u32, values: Value, pauses: [PauseHap; 2]) -> Value {
    let pauses: Vec<Value> = pauses
        .iter()
        .enumerate()
        .map(|(i, p)| {
            json!({
                "suspendSec": p.suspend_sec,
                "haps": [{ "t": p.t, "duration": p.duration, "cycle": i, "v": 0, "event": i }]
            })
        })
        .collect();
    json!({
        "schema": 2, "sampleRate": 44100, "frames": frames, "seed": 0, "cps": 0.5,
        "channels": { "count": channels, "multiChannelOrbits": channels > 2 },
        "orbits": [{ "orbit": 1, "channels": [0, 1] }],
        "pauses": pauses,
        "values": values
    })
}
