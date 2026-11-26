//! The per-song performance budget on the render corpus, for checking a regression later.
//!
//! Ignored by default: it needs a release build on a quiet machine. Run it explicitly:
//!
//! ```text
//! cargo test --release -p strudeland-host --test perf_budget -- --ignored --nocapture
//! ```
//!
//! It plans the largest corpus fixture (the widest manifest window; ties go to the larger job
//! of a first run) over its full manifest window as one chunk (a single-chunk `plan_render`)
//! three times, each on a fresh `Host`, and asserts the median is within
//! `BUDGET.full_song_max_ms`. It times that one call, not a production render's plan of every
//! chunk.

mod common;

use std::time::Instant;

use common::{corpus_song, job, repo_root, single_chunk};
use serde_json::Value;
use strudeland_host::bench::{BUDGET, RUNS, median};
use strudeland_host::{Host, Layout, LookbackSec, Mode, RenderPlanRequest, SingleChunk};

struct Fixture {
    name: String,
    code: String,
    from_bar: u32,
    to_bar: u32,
    request: RenderPlanRequest,
}

fn fixtures() -> Vec<Fixture> {
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("tests/render/corpus.json")).unwrap(),
    )
    .unwrap();
    manifest["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let window = f["windowBars"].as_array().unwrap();
            let bar = |i: usize| u32::try_from(window[i].as_u64().unwrap()).unwrap();
            Fixture {
                name: f["name"].as_str().unwrap().to_string(),
                code: corpus_song(f["file"].as_str().unwrap()),
                from_bar: bar(0),
                to_bar: bar(1),
                request: single_chunk(
                    bar(0),
                    bar(1),
                    serde_json::from_value::<Layout>(f["layout"].clone()).unwrap(),
                    SingleChunk {
                        overlap_bars: 0,
                        lookback_sec: LookbackSec::Seconds(0.0),
                        mode: Mode::Mix,
                    },
                ),
            }
        })
        .collect()
}

/// The widest window; among equally wide ones, the larger first-run job.
fn largest(fixtures: Vec<Fixture>) -> Fixture {
    let width = |f: &Fixture| f.to_bar - f.from_bar;
    let widest = fixtures.iter().map(width).max().unwrap();
    let mut candidates: Vec<Fixture> = fixtures
        .into_iter()
        .filter(|f| width(f) == widest)
        .collect();
    if candidates.len() == 1 {
        return candidates.pop().unwrap();
    }
    let host = Host::new().unwrap();
    candidates
        .into_iter()
        .map(|f| {
            let bytes = job(&host.plan_render(&f.code, &f.request).unwrap()).len();
            (bytes, f)
        })
        .max_by_key(|(bytes, _)| *bytes)
        .unwrap()
        .1
}

#[test]
#[ignore = "needs a release build on a quiet machine: cargo test --release -p strudeland-host --test perf_budget -- --ignored --nocapture"]
fn the_largest_corpus_fixture_plans_within_the_per_song_budget() {
    if cfg!(debug_assertions) {
        panic!("the performance budget is judged in release: run with --release");
    }
    let fixture = largest(fixtures());
    let mut times = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let host = Host::new().unwrap();
        let start = Instant::now();
        host.plan_render(&fixture.code, &fixture.request)
            .unwrap_or_else(|e| panic!("{}: {e}", fixture.name));
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let median_ms = median(&times).unwrap();
    println!(
        "perf_budget: {} bars {}..{}: runs {times:.1?} ms, median {median_ms:.1} ms (limit {} ms)",
        fixture.name, fixture.from_bar, fixture.to_bar, BUDGET.full_song_max_ms
    );
    assert!(
        median_ms <= BUDGET.full_song_max_ms,
        "{}: median {median_ms:.1} ms is over the {} ms budget",
        fixture.name,
        BUDGET.full_song_max_ms
    );
}
