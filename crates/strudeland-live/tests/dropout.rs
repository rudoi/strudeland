//! The dropout test plays the synthetic live-pressure job through the real-time fake output
//! at 48 kHz (so through the resampler) while busy threads load every core. Batches are sent
//! the way the webview sends them (one per 100 ms window, 100 ms ahead). The last `Stats`
//! must show zero underruns, zero late haps and zero skipped haps.
//!
//! `dropout_short` (3 s, up to two busy threads) runs in `cargo test`. Run `dropout_full`
//! (30 s under load) and `dropout_long_reverb` (sustained reverb) on a quiet machine:
//! `cargo test --release -p strudeland-live --test dropout -- --ignored --nocapture --test-threads=1`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value as Json;
use strudeland_engine::OrbitSpec;
use strudeland_live::testing::{FakeOutput, Pace};
use strudeland_live::{Batch, Event, HapEntry, OutputInfo, Player, Stats, ValueEntry};

const FIXTURE: &str = include_str!("data/live-pressure.job.json");
/// Engine seconds before the first window, so the first batch is not already due.
const LEAD_IN: f64 = 0.3;
/// The cyclist's interval and latency.
const WINDOW: f64 = 0.1;
const LATENCY: f64 = 0.1;

struct Hap {
    t: f64,
    duration: f64,
    cycle: f64,
    v: usize,
}

struct Job {
    values: Vec<Json>,
    haps: Vec<Hap>,
    cps: f64,
}

fn job() -> Job {
    let job: Json = serde_json::from_str(FIXTURE).expect("live-pressure job parses");
    let mut haps: Vec<Hap> = job["pauses"]
        .as_array()
        .expect("pauses")
        .iter()
        .flat_map(|p| p["haps"].as_array().expect("haps").iter())
        .map(|h| Hap {
            t: h["t"].as_f64().unwrap(),
            duration: h["duration"].as_f64().unwrap(),
            cycle: h["cycle"].as_f64().unwrap(),
            v: h["v"].as_u64().unwrap() as usize,
        })
        .collect();
    haps.sort_by(|a, b| a.t.total_cmp(&b.t));
    Job {
        values: job["values"].as_array().expect("values").clone(),
        haps,
        cps: job["cps"].as_f64().unwrap(),
    }
}

/// The start of the `len`-second span holding the most haps (starting at a hap).
fn densest(haps: &[Hap], len: f64) -> f64 {
    let mut best = (0, haps[0].t);
    let mut end = 0;
    for (i, h) in haps.iter().enumerate() {
        while end < haps.len() && haps[end].t < h.t + len {
            end += 1;
        }
        if end - i > best.0 {
            best = (end - i, h.t);
        }
    }
    best.1
}

/// The span's batches, one per 100 ms window, with each window's start in engine seconds. Values
/// are sent on first use (indices 0, 1, 2, ... in that order) with an `OrbitSpec` the first
/// time their orbit is seen.
fn batches(job: &Job, seconds: f64) -> Vec<(f64, Batch)> {
    let t0 = densest(&job.haps, seconds);
    let span: Vec<&Hap> = job
        .haps
        .iter()
        .filter(|h| h.t >= t0 && h.t < t0 + seconds)
        .collect();
    let mut index_of: HashMap<usize, u32> = HashMap::new();
    let mut orbits_seen: Vec<f64> = Vec::new();
    let windows = (seconds / WINDOW).ceil() as usize;
    let mut out = Vec::new();
    for w in 0..windows {
        let start = LEAD_IN + w as f64 * WINDOW;
        // Compute the shared boundary the same way for both adjacent batches.
        let end = LEAD_IN + (w + 1) as f64 * WINDOW;
        let mut batch = Batch {
            values: Vec::new(),
            haps: Vec::new(),
            cps: job.cps,
        };
        for h in &span {
            let t = h.t - t0 + LEAD_IN;
            if t < start || t >= end {
                continue;
            }
            let next = index_of.len() as u32;
            let index = *index_of.entry(h.v).or_insert_with(|| {
                let value = job.values[h.v].clone();
                let orbit = value.get("orbit").and_then(Json::as_f64).unwrap_or(1.0);
                let spec = (!orbits_seen.contains(&orbit)).then(|| {
                    orbits_seen.push(orbit);
                    OrbitSpec {
                        orbit,
                        channels: vec![0, 1],
                    }
                });
                batch.values.push(ValueEntry(next, value, spec));
                next
            });
            batch
                .haps
                .push(HapEntry(t, h.duration, h.cycle, index, None));
        }
        if !batch.haps.is_empty() {
            out.push((start, batch));
        }
    }
    assert_eq!(
        out.iter().map(|(_, batch)| batch.haps.len()).sum::<usize>(),
        span.len(),
        "every hap in the selected window belongs to one batch"
    );
    out
}

fn busy_threads(n: usize) -> (Arc<AtomicBool>, Vec<std::thread::JoinHandle<()>>) {
    let stop = Arc::new(AtomicBool::new(false));
    let threads = (0..n)
        .map(|_| {
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut x: u64 = 1;
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..10_000 {
                        x = std::hint::black_box(
                            x.wrapping_mul(6364136223846793005).wrapping_add(1),
                        );
                    }
                }
                std::hint::black_box(x);
            })
        })
        .collect();
    (stop, threads)
}

fn run(seconds: f64, busy: usize) -> Stats {
    let job = job();
    let batches = batches(&job, seconds);
    let total_haps: usize = batches.iter().map(|(_, b)| b.haps.len()).sum();
    let (stop_busy, busy) = busy_threads(busy);
    let (out, _handle) = FakeOutput::new(
        OutputInfo {
            sample_rate: 48000,
            buffer_frames: Some(512),
        },
        Pace::RealTime,
    );
    let (player, events) = Player::start(Box::new(out), 0).expect("player starts");
    let wall0 = Instant::now();
    let n = batches.len() as u64;
    for (start, batch) in batches {
        let due = wall0 + Duration::from_secs_f64(start - LATENCY);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        player.schedule(batch);
    }
    // the first stats after the span has played
    let end = wall0 + Duration::from_secs_f64(LEAD_IN + seconds + 0.2);
    let mut last = None;
    let mut errors = Vec::new();
    let deadline = end + Duration::from_secs(3);
    while Instant::now() < deadline {
        match events.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Stats(s)) => {
                let done = Instant::now() >= end && s.batches == n;
                last = Some(s);
                if done {
                    break;
                }
            }
            Ok(Event::Error { message }) => errors.push(message),
            _ => {}
        }
    }
    player.stop();
    stop_busy.store(true, Ordering::Relaxed);
    for t in busy {
        let _ = t.join();
    }
    let stats = last.expect("a stats event");
    let refusals = strudeland_live::realtime_refusals();
    println!(
        "dropout {seconds} s: {n} batches, {total_haps} haps sent; stats {stats:?}; {} error events {errors:?}; {refusals} time-constraint refusals",
        errors.len()
    );
    #[cfg(target_vendor = "apple")]
    assert_eq!(refusals, 0, "the engine and output threads run real-time");
    assert_eq!(stats.batches, n, "every batch arrived");
    stats
}

fn check(stats: &Stats) {
    assert_eq!(stats.underruns, 0, "{stats:?}");
    assert_eq!(stats.late_haps, 0, "{stats:?}");
    assert_eq!(stats.skipped_haps, 0, "{stats:?}");
}

#[test]
fn the_batches_cover_the_span() {
    let job = job();
    let b = batches(&job, 3.0);
    assert!(!b.is_empty());
    // values precede their haps and are sent once, in index order
    let mut next = 0;
    for (start, batch) in &b {
        assert!(batch.validate().is_ok());
        for ValueEntry(i, _, _) in &batch.values {
            assert_eq!(*i, next);
            next += 1;
        }
        for HapEntry(t, _, _, i, _) in &batch.haps {
            assert!(*i < next);
            assert!(*t >= *start && *t < start + WINDOW);
        }
    }
    assert!(
        b[0].1.values[0].2.is_some(),
        "the first value brings its orbit"
    );
}

#[test]
fn dropout_short() {
    // Apple Silicon uses the calibrated Mach real-time policy. On other hosts reserve
    // execution capacity for the normal-priority engine, device and batch producer
    // before adding synthetic contention. The zero-dropout assertion stays the same.
    let busy = if cfg!(target_vendor = "apple") {
        2
    } else {
        std::thread::available_parallelism().map_or(0, |n| n.get().saturating_sub(3).min(2))
    };
    check(&run(3.0, busy));
}

#[test]
#[ignore = "30 s under full load: run with --release --ignored --nocapture on a quiet machine"]
fn dropout_full() {
    let busy = std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(1));
    check(&run(30.0, busy));
}

#[test]
#[ignore = "real-time 48 kHz convolution budget: run with --release --ignored --nocapture"]
fn dropout_long_reverb() {
    const RATE: u32 = 48_000;
    const BLOCK: u32 = 512;
    const FREQUENCIES: [f64; 4] = [137.0, 178.0, 219.0, 260.0];
    const ROOM_SIZE: f64 = 22.0;
    // Every IR must land in the first two seconds. Allow its 33-second history to fill.
    const SECONDS: f64 = 36.0;
    const STATS_GRACE: Duration = Duration::from_secs(3);
    let (out, handle) = FakeOutput::new(
        OutputInfo {
            sample_rate: RATE,
            buffer_frames: Some(BLOCK),
        },
        Pace::RealTime,
    );
    let (player, events) = Player::start(Box::new(out), 0).expect("player starts");
    let mut batch = Batch {
        values: vec![],
        haps: vec![],
        cps: 1.0,
    };
    for (i, freq) in FREQUENCIES.into_iter().enumerate() {
        let orbit = i as f64;
        let value = serde_json::json!({
            "s": "sine", "freq": freq, "gain": 0.06,
            "dry": 0.0, "room": 0.8, "roomsize": ROOM_SIZE, "orbit": orbit,
        });
        batch.values.push(ValueEntry(
            i as u32,
            value,
            Some(OrbitSpec {
                orbit,
                channels: vec![0, 1],
            }),
        ));
        batch.haps.push(HapEntry(
            LEAD_IN,
            SECONDS + STATS_GRACE.as_secs_f64(),
            LEAD_IN,
            i as u32,
            None,
        ));
    }
    player.schedule(batch);

    let mut headroom = f64::INFINITY;
    let mut stats = None;
    let mut errors = Vec::new();
    let end = Instant::now() + Duration::from_secs_f64(SECONDS);
    let deadline = end + STATS_GRACE;
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match events.recv_timeout(remaining) {
            Ok(Event::Stats(s)) if s.haps == FREQUENCIES.len() as u64 => {
                headroom = headroom.min(s.render_headroom_ms);
                if Instant::now() >= end {
                    stats = Some(s);
                    break;
                }
            }
            Ok(Event::Error { message }) => errors.push(message),
            Err(_) => break,
            _ => {}
        }
    }
    player.stop();
    let stats = stats.expect("stats reported scheduled haps through the end of playback");
    let samples = handle.pulled();
    println!("long reverb: {stats:?}; minimum fill {headroom:.2} ms; {errors:?}");
    assert!(errors.is_empty(), "{errors:?}");
    check(&stats);
    assert!(
        headroom >= f64::from(BLOCK) * 1000.0 / f64::from(RATE),
        "{headroom:.2} ms"
    );

    // Check wet-only output during startup and before the stop fade. Every orbit must play.
    let second = RATE as usize * 2;
    let end = samples.len() - RATE as usize / 2;
    for (label, wet) in [
        ("startup", &samples[second..2 * second]),
        ("sustained", &samples[end - second..end]),
    ] {
        for freq in FREQUENCIES {
            let (re, im) =
                wet.chunks_exact(2)
                    .enumerate()
                    .fold((0.0, 0.0), |(re, im), (i, stereo)| {
                        let phase = std::f64::consts::TAU * freq * i as f64 / f64::from(RATE);
                        let (sin, cos) = phase.sin_cos();
                        let sample = f64::from(stereo[0]);
                        (re + sample * cos, im + sample * sin)
                    });
            assert!(
                re.hypot(im) / f64::from(RATE) > 1e-5,
                "{label} wet tone {freq} Hz"
            );
        }
    }
}
