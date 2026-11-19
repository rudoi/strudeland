//! The compute budget: a minute of synthetic 48 kHz audio through `transcribe` (resampling
//! included), the median of three runs under `PERF_BUDGET_SECONDS_PER_MINUTE`.
//!
//! Ignored by default: it needs a release build on a quiet machine. Run it explicitly:
//!
//! ```text
//! cargo test --release -p strudeland-transcribe --features eval --test perf_budget -- --ignored --nocapture
//! ```

use std::time::Instant;

use strudeland_transcribe::consts::PERF_BUDGET_SECONDS_PER_MINUTE;
use strudeland_transcribe::eval::cases::{Tier, tier_cases};
use strudeland_transcribe::eval::synth::DEFAULT_RATE;
use strudeland_transcribe::{GridHint, Hinted, Options, transcribe};

const RUNS: usize = 3;
const SECONDS: f64 = 60.0;

/// 60 s of the vibrato tier's voice (the hardest segmentation): its cases rendered back to back
/// and cut to a minute.
fn minute() -> Vec<f32> {
    let want = (SECONDS * f64::from(DEFAULT_RATE)) as usize;
    let mut samples = Vec::with_capacity(want);
    for case in tier_cases(Tier::VibratoGlide).iter().cycle() {
        samples.extend(case.case.render(DEFAULT_RATE));
        if samples.len() >= want {
            break;
        }
    }
    samples.truncate(want);
    samples
}

#[test]
#[ignore = "release only: cargo test --release -p strudeland-transcribe --features eval --test perf_budget -- --ignored --nocapture"]
fn perf_budget() {
    let samples = minute();
    assert_eq!(samples.len() as f64, SECONDS * f64::from(DEFAULT_RATE));
    let hint = GridHint {
        bpm: Some(Hinted::sidecar(110.0)),
        beats_per_bar: Some(Hinted::sidecar(4.0)),
        origin_seconds: Some(Hinted::sidecar(2.0)),
        ..GridHint::default()
    };
    let mut times = Vec::with_capacity(RUNS);
    for run in 0..RUNS {
        let start = Instant::now();
        let t =
            transcribe(&samples, DEFAULT_RATE, &hint, &Options::default()).expect("transcribes");
        let elapsed = start.elapsed().as_secs_f64();
        assert!(!t.notes.is_empty(), "run {run}: notes");
        println!(
            "run {run}: {elapsed:.3} s for {SECONDS} s of audio, {} notes",
            t.notes.len()
        );
        times.push(elapsed);
    }
    times.sort_by(f64::total_cmp);
    let median = times[RUNS / 2];
    let per_minute = median * 60.0 / SECONDS;
    println!(
        "median {median:.3} s: {per_minute:.3} s per minute (budget {PERF_BUDGET_SECONDS_PER_MINUTE} s)"
    );
    assert!(
        per_minute < PERF_BUDGET_SECONDS_PER_MINUTE,
        "{per_minute:.3} s per minute of audio, over the {PERF_BUDGET_SECONDS_PER_MINUTE} s budget"
    );
}
