//! The Rust joiner the pipeline renders into (`Joiner::for_plans` + `Joiner::place_rendered`)
//! against the TypeScript `assembleWindow`: the vectors in `tests/vectors/joiner.json` (from
//! `bun run render:vectors`) hold each chunk's planned `contentFrames` and buffer `frames`, and
//! each case's frame count and the FNV-1a 64 hash of the joined audio; this test rebuilds the
//! same synthetic chunk buffers from those counts (computing none), places them in several orders
//! (the workers finish in any order) and compares.

use serde::Deserialize;
use strudeland_host::{ChunkWindow, Layout, PlannedWindow};
use strudeland_render::join::Joiner;
use strudeland_render::testing::window_frames;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Window {
    from_bar: u32,
    to_bar: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chunk {
    from_bar: u32,
    to_bar: u32,
    window_start_frame: u64,
    overlap_bars: u32,
    content_frames: u64,
    frames: u64,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    cps: f64,
    window: Window,
    chunks: Vec<Chunk>,
    frames: u64,
    fnv1a64: String,
}

#[derive(Deserialize)]
struct File {
    schema: u32,
    cases: Vec<Case>,
}

/// Sample `i` of channel `c` of chunk `k` (`joinerSample` in the generator).
fn sample(k: u64, c: u64, i: u64) -> f32 {
    ((k * 7919 + c * 104_729 + i * 31) % 65_521) as f32 / 65_536.0
}

fn fnv1a64(channels: &[Vec<f32>]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for ch in channels {
        for x in ch {
            for b in x.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    format!("{h:016x}")
}

#[test]
fn the_rust_joiner_matches_assemble_window() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/joiner.json");
    let file: File = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(file.schema, 1);
    assert!(file.cases.len() >= 6, "{} cases", file.cases.len());
    for case in &file.cases {
        let name = &case.name;
        let (plans, buffers): (Vec<ChunkWindow>, Vec<Vec<Vec<f32>>>) = case
            .chunks
            .iter()
            .enumerate()
            .map(|(k, ch)| {
                let frames = ch.frames;
                let plan = ChunkWindow {
                    c0: ch.from_bar,
                    c1: ch.to_bar + ch.overlap_bars,
                    lookback_sec_used: ch.window_start_frame as f64 / 44_100.0,
                    content_frames: ch.content_frames,
                    overlap_frames: None,
                    window_start_frame: ch.window_start_frame,
                    buffer_start_sec: 0.0,
                    frames,
                    from_bar: ch.from_bar,
                    to_bar: ch.to_bar,
                    lookback_sec: ch.window_start_frame as f64 / 44_100.0,
                };
                let channels = (0..2)
                    .map(|c| (0..frames).map(|i| sample(k as u64, c, i)).collect())
                    .collect();
                (plan, channels)
            })
            .unzip();
        let window = PlannedWindow {
            from_bar: case.window.from_bar,
            to_bar: case.window.to_bar,
            layout: Layout::Linear,
        };
        assert_eq!(
            window_frames(case.window.from_bar, case.window.to_bar, case.cps),
            case.frames,
            "{name}: window_frames"
        );
        let n = plans.len();
        // In order, reversed, and odd chunks first.
        let orders: [Vec<usize>; 3] = [
            (0..n).collect(),
            (0..n).rev().collect(),
            (0..n)
                .filter(|i| i % 2 == 1)
                .chain((0..n).filter(|i| i % 2 == 0))
                .collect(),
        ];
        for order in orders {
            let mut joiner = Joiner::for_plans(&window, &plans, 2, case.frames, 0)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            for &i in &order {
                joiner
                    .place_rendered(i, &plans[i], &buffers[i])
                    .unwrap_or_else(|e| panic!("{name} {order:?}: {e}"));
            }
            let joined = joiner
                .finish()
                .unwrap_or_else(|e| panic!("{name} {order:?}: {e}"));
            assert_eq!(joined.len(), 2, "{name}");
            assert_eq!(joined[0].len() as u64, case.frames, "{name}: frames");
            assert_eq!(fnv1a64(&joined), case.fnv1a64, "{name} {order:?}: hash");
        }
    }
}

#[test]
fn the_hash_detects_a_one_frame_shift() {
    // A guard on the guard: moving one chunk boundary by one frame changes the hash.
    let a = vec![(0..100).map(|i| sample(0, 0, i)).collect::<Vec<f32>>()];
    let b = vec![(1..101).map(|i| sample(0, 0, i)).collect::<Vec<f32>>()];
    assert_ne!(fnv1a64(&a), fnv1a64(&b));
    assert_eq!(fnv1a64(&[]), "cbf29ce484222325");
}
