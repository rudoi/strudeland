//! The EBU Tech 3341 (2023) "minimum requirements" test signals that a stereo, file-based
//! integrated-loudness and true-peak measure can be checked with, synthesised here (the reference
//! WAVs are not needed): loudness cases 1 to 5 (integrated loudness within +-0.1 LU) and the
//! true-peak cases 15 to 23 (within +0.2/-0.4 dB of the expected level), at 44.1 and 48 kHz.
//! Cases 6 (5.0 channel weights), 7 and 8 (programme material) and 9 to 14 (momentary and
//! short-term readings) do not apply to a stereo integrated measure.

use std::f64::consts::PI;

use strudeland_render::level::{integrated_lufs, true_peak_db};

const RATES: [u32; 2] = [44_100, 48_000];

/// `seconds` of a 1 kHz sine at `db` dBFS peak, continuing the phase from frame `start`.
fn tone(sr: u32, start: usize, seconds: f64, db: f64) -> Vec<f64> {
    let amp = 10f64.powf(db / 20.0);
    let frames = (seconds * f64::from(sr)).round() as usize;
    (start..start + frames)
        .map(|i| amp * (2.0 * PI * 1000.0 * i as f64 / f64::from(sr)).sin())
        .collect()
}

/// Consecutive 1 kHz tones `(seconds, dBFS)`, the same on both channels.
fn tones(sr: u32, parts: &[(f64, f64)]) -> Vec<Vec<f32>> {
    let mut x: Vec<f64> = Vec::new();
    for &(s, db) in parts {
        let next = tone(sr, x.len(), s, db);
        x.extend(next);
    }
    let ch: Vec<f32> = x.into_iter().map(|v| v as f32).collect();
    vec![ch.clone(), ch]
}

fn check_loudness(case: u32, parts: &[(f64, f64)], want: f64) {
    for sr in RATES {
        let got = integrated_lufs(&tones(sr, parts), sr);
        assert!(
            (got - want).abs() <= 0.1,
            "case {case} at {sr} Hz: {got} LUFS, want {want} +-0.1"
        );
    }
}

#[test]
fn level_ebu_case_1_sine_at_minus_23() {
    check_loudness(1, &[(20.0, -23.0)], -23.0);
}

#[test]
fn level_ebu_case_2_sine_at_minus_33() {
    check_loudness(2, &[(20.0, -33.0)], -33.0);
}

#[test]
fn level_ebu_case_3_relative_gate() {
    check_loudness(3, &[(10.0, -36.0), (60.0, -23.0), (10.0, -36.0)], -23.0);
}

#[test]
fn level_ebu_case_4_absolute_and_relative_gates() {
    check_loudness(
        4,
        &[
            (10.0, -72.0),
            (10.0, -36.0),
            (60.0, -23.0),
            (10.0, -36.0),
            (10.0, -72.0),
        ],
        -23.0,
    );
}

#[test]
fn level_ebu_case_5_louder_middle() {
    check_loudness(5, &[(20.0, -26.0), (20.1, -20.0), (20.0, -26.0)], -23.0);
}

/// A raised-cosine fade-in and fade-out of `fade` frames.
fn taper(x: &mut [f64], fade: usize) {
    let n = x.len();
    for (i, v) in x.iter_mut().enumerate() {
        let edge = i.min(n - 1 - i);
        if edge < fade {
            *v *= 0.5 - 0.5 * (PI * edge as f64 / fade as f64).cos();
        }
    }
}

fn stereo(x: &[f64]) -> Vec<Vec<f32>> {
    let ch: Vec<f32> = x.iter().map(|&v| v as f32).collect();
    vec![ch.clone(), ch]
}

fn check_true_peak(case: &str, x: &[Vec<f32>], want: f64) {
    let got = true_peak_db(x);
    assert!(
        got >= want - 0.4 && got <= want + 0.2,
        "case {case}: {got} dBTP, want {want} +0.2/-0.4"
    );
}

/// Cases 15 to 19: a stereo sine at `fs / div`, `amp` FFS, starting at `phase_deg`, one second with
/// 10 ms fades.
fn tapered_sine(sr: u32, div: f64, amp: f64, phase_deg: f64) -> Vec<Vec<f32>> {
    let mut x: Vec<f64> = (0..sr as usize)
        .map(|i| amp * (2.0 * PI * i as f64 / div + phase_deg.to_radians()).sin())
        .collect();
    taper(&mut x, (0.010 * f64::from(sr)).round() as usize);
    stereo(&x)
}

#[test]
fn level_ebu_cases_15_to_19_sines() {
    for sr in RATES {
        for (case, div, amp, phase, want) in [
            (15, 4.0, 0.5, 0.0, -6.0),
            (16, 4.0, 0.5, 45.0, -6.0),
            (17, 6.0, 0.5, 60.0, -6.0),
            (18, 8.0, 0.5, 67.5, -6.0),
            (19, 4.0, 1.41, 45.0, 3.0),
        ] {
            check_true_peak(
                &format!("{case} at {sr} Hz"),
                &tapered_sine(sr, div, amp, phase),
                want,
            );
        }
    }
}

/// A windowed-sinc low-pass (Blackman window, `taps` odd) with cutoff `fc` in cycles per sample,
/// unity gain at DC.
fn lowpass(taps: usize, fc: f64) -> Vec<f64> {
    let m = (taps - 1) as f64;
    let mut h: Vec<f64> = (0..taps)
        .map(|n| {
            let t = n as f64 - m / 2.0;
            let sinc = if t == 0.0 {
                2.0 * fc
            } else {
                (2.0 * PI * fc * t).sin() / (PI * t)
            };
            let w = 0.42 - 0.5 * (2.0 * PI * n as f64 / m).cos()
                + 0.08 * (4.0 * PI * n as f64 / m).cos();
            sinc * w
        })
        .collect();
    let sum: f64 = h.iter().sum();
    for v in &mut h {
        *v /= sum;
    }
    h
}

/// Cases 20 to 23: at 4x the rate, a 0.5 FFS sine at fs/6 containing one period of a 1.0 FFS sine
/// at fs/4, continuous in phase on both sides (both start and end at an upward zero crossing),
/// tapered, low-pass filtered and downsampled with `offset` samples (at the 4x rate).
fn burst(offset: usize) -> Vec<Vec<f32>> {
    // At 4x: fs/6 is 24 samples a period, fs/4 is 16.
    let (lead, tail) = (24 * 400, 24 * 400);
    let mut x: Vec<f64> = Vec::with_capacity(lead + 16 + tail);
    x.extend((0..lead).map(|n| 0.5 * (2.0 * PI * n as f64 / 24.0).sin()));
    x.extend((0..16).map(|n| (2.0 * PI * n as f64 / 16.0).sin()));
    x.extend((0..tail).map(|n| 0.5 * (2.0 * PI * n as f64 / 24.0).sin()));
    taper(&mut x, 24 * 20);
    // Anti-aliasing for the decimation by 4: cutoff at the target rate's Nyquist (1/8 of the 4x
    // rate), 511 taps (zero phase: the output is centred).
    let h = lowpass(511, 0.125);
    let half = h.len() / 2;
    let filtered: Vec<f64> = (0..x.len())
        .map(|n| {
            h.iter()
                .enumerate()
                .map(|(k, c)| {
                    let idx = n as isize + half as isize - k as isize;
                    if idx >= 0 && (idx as usize) < x.len() {
                        c * x[idx as usize]
                    } else {
                        0.0
                    }
                })
                .sum()
        })
        .collect();
    let down: Vec<f64> = filtered.iter().skip(offset).step_by(4).copied().collect();
    stereo(&down)
}

#[test]
fn level_ebu_cases_20_to_23_single_period_burst() {
    for (case, offset) in [(20, 0), (21, 1), (22, 2), (23, 3)] {
        // The signal is defined relative to fs, so one synthesis serves every rate.
        check_true_peak(&case.to_string(), &burst(offset), 0.0);
    }
}
