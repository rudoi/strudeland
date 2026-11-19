//! The Rust integrated loudness against the TypeScript `integratedLufs`: `tests/vectors/
//! loudness.json` (from `bun run render:vectors`) holds fixed signals as recipes and the TypeScript
//! result for each; this test rebuilds every signal from its recipe and compares within 1e-6 LU
//! (`null` is `-inf` and must match exactly).

use serde::Deserialize;
use strudeland_engine::Mulberry32;
use strudeland_render::level::integrated_lufs;

/// Absolute tolerance, in LU: both sides are f64 on f32 input; only libm and the summation order
/// differ.
const TOLERANCE_LU: f64 = 1e-6;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Part {
    Sine {
        hz: f64,
        amp: f64,
        from: Option<usize>,
        to: Option<usize>,
    },
    Noise {
        amp: f64,
        seed: u32,
        from: Option<usize>,
        to: Option<usize>,
    },
}

#[derive(Deserialize)]
struct Channel {
    frames: usize,
    parts: Vec<Part>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Signal {
    name: String,
    sample_rate: u32,
    channels: Vec<Channel>,
    lufs: Option<f64>,
}

#[derive(Deserialize)]
struct File {
    schema: u32,
    signals: Vec<Signal>,
}

/// `synthChannel` of the generator.
fn synth(c: &Channel, sample_rate: u32) -> Vec<f32> {
    let sr = f64::from(sample_rate);
    let mut acc = vec![0.0f64; c.frames];
    for p in &c.parts {
        match *p {
            Part::Sine { hz, amp, from, to } => {
                let (from, to) = (from.unwrap_or(0), to.unwrap_or(c.frames).min(c.frames));
                for (i, v) in acc.iter_mut().enumerate().take(to).skip(from) {
                    *v += amp * (2.0 * std::f64::consts::PI * hz * i as f64 / sr).sin();
                }
            }
            Part::Noise {
                amp,
                seed,
                from,
                to,
            } => {
                let (from, to) = (from.unwrap_or(0), to.unwrap_or(c.frames).min(c.frames));
                let mut rng = Mulberry32::new(seed);
                for v in acc.iter_mut().take(to).skip(from) {
                    *v += amp * (2.0 * rng.next() - 1.0);
                }
            }
        }
    }
    acc.into_iter().map(|v| v as f32).collect()
}

#[test]
fn loudness_matches_the_typescript_vectors() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/loudness.json");
    let text = std::fs::read_to_string(path).expect("read loudness.json");
    let file: File = serde_json::from_str(&text).expect("parse loudness.json");
    assert_eq!(file.schema, 1);
    assert!(file.signals.len() >= 12);
    for s in &file.signals {
        let channels: Vec<Vec<f32>> = s.channels.iter().map(|c| synth(c, s.sample_rate)).collect();
        let got = integrated_lufs(&channels, s.sample_rate);
        match s.lufs {
            None => assert_eq!(got, f64::NEG_INFINITY, "{}: want -inf, got {got}", s.name),
            Some(want) => assert!(
                (got - want).abs() <= TOLERANCE_LU,
                "{}: got {got}, want {want} (diff {})",
                s.name,
                got - want
            ),
        }
    }
}
