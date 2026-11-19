//! The Rust half of the IPC benchmark: what decoding a `playback_schedule` batch costs.
//!
//! Run by hand: `cargo test --release -p strudeland-live --test ipc_decode -- --ignored
//! --nocapture`.

use std::time::Instant;

use serde_json::Value as Json;
use strudeland_engine::{OrbitSpec, Value};
use strudeland_live::{Batch, HapEntry, ValueEntry};

const FIXTURE: &str = include_str!("data/live-pressure.job.json");
const REPETITIONS: usize = 200;

struct Fixture {
    values: Vec<Json>,
    haps: Vec<(f64, f64, f64, usize)>,
    cps: f64,
    orbit: OrbitSpec,
}

fn fixture() -> Fixture {
    let job: Json = serde_json::from_str(FIXTURE).expect("live-pressure job parses");
    let values = job["values"].as_array().expect("values").clone();
    let mut haps = Vec::new();
    for pause in job["pauses"].as_array().expect("pauses") {
        for h in pause["haps"].as_array().expect("haps") {
            haps.push((
                h["t"].as_f64().unwrap(),
                h["duration"].as_f64().unwrap(),
                h["cycle"].as_f64().unwrap(),
                h["v"].as_u64().unwrap() as usize,
            ));
        }
    }
    let orbit = serde_json::from_value(job["orbits"][0].clone()).expect("orbit spec");
    Fixture {
        values,
        haps,
        cps: job["cps"].as_f64().unwrap(),
        orbit,
    }
}

/// A batch of `n` haps. With `new_values`, every hap brings its own value entry.
fn make_batch(f: &Fixture, n: usize, new_values: bool) -> Batch {
    let mut values = Vec::new();
    let mut haps = Vec::new();
    for i in 0..n {
        let (t, d, c, v) = f.haps[i % f.haps.len()];
        let index = if new_values { i } else { v } as u32;
        if new_values {
            let orbit = (i == 0).then(|| f.orbit.clone());
            values.push(ValueEntry(index, f.values[v].clone(), orbit));
        }
        haps.push(HapEntry(t + i as f64 * 0.01, d, c, index));
    }
    Batch {
        values,
        haps,
        cps: f.cps,
    }
}

fn decode(text: &str) -> usize {
    let batch: Batch = serde_json::from_str(text).expect("batch parses");
    let mut n = 0;
    for ValueEntry(_, json, _) in batch.values {
        match Value::from_json(json).expect("value decodes") {
            Value::Obj(m) => n += m.len(),
            _ => panic!("not a control map"),
        }
    }
    n + batch.haps.len()
}

#[test]
fn a_batch_decodes_to_what_was_built() {
    let f = fixture();
    let batch = make_batch(&f, 16, true);
    let text = serde_json::to_string(&batch).unwrap();
    let back: Batch = serde_json::from_str(&text).unwrap();
    assert_eq!(back, batch);
    assert!(back.validate().is_ok());
    assert!(decode(&text) > 16);
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn decode_sweep() {
    let f = fixture();
    println!("haps, variant, bytes, p50 us, max us");
    for n in [1usize, 16, 64, 256] {
        for (name, new_values) in [("new values", true), ("no new values", false)] {
            let text = serde_json::to_string(&make_batch(&f, n, new_values)).unwrap();
            let mut times: Vec<u64> = (0..REPETITIONS)
                .map(|_| {
                    let start = Instant::now();
                    std::hint::black_box(decode(std::hint::black_box(&text)));
                    start.elapsed().as_micros() as u64
                })
                .collect();
            times.sort_unstable();
            println!(
                "{n}, {name}, {}, {}, {}",
                text.len(),
                times[times.len() / 2],
                times[times.len() - 1]
            );
        }
    }
}
