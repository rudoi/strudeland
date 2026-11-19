//! The true-peak limiter: a gain computed from the 4x-oversampled peaks, with the lookahead taken
//! from future frames (no delay line, so the output length equals the input length exactly), an
//! instant attack over the lookahead window and an exponential release. When no oversampled point
//! crosses the ceiling the input comes back bit-identical.

use super::true_peak::{TAPS_PER_PHASE, peak_of_frames};

/// Lookahead, in seconds (66 frames at 44.1 kHz).
pub const LOOKAHEAD_SEC: f64 = 0.0015;
/// Release time constant, in seconds.
pub const RELEASE_SEC: f64 = 0.050;
/// Frames on each side of a peak that take its required gain: the true-peak filter interpolates
/// over 12 input samples.
pub const SPREAD: usize = TAPS_PER_PHASE / 2;
/// Limiter passes at most (the first, and up to two tightened ones).
pub const MAX_PASSES: u32 = 3;
/// Extra margin, in dB, a tightened pass takes below the measured overshoot.
pub const TIGHTEN_MARGIN_DB: f64 = 0.01;

/// Lookahead in frames: `round(LOOKAHEAD_SEC * sample_rate)`, at least 1.
pub fn lookahead_frames(sample_rate: u32) -> usize {
    ((LOOKAHEAD_SEC * f64::from(sample_rate)).round() as usize).max(1)
}

/// `x * gain`, one rounding to f32.
pub(crate) fn apply_one(x: f32, gain: f64) -> f32 {
    (f64::from(x) * gain) as f32
}

/// Frames a gain attenuates (below 1).
pub(crate) fn touched(gain: &[f64]) -> u64 {
    gain.iter().filter(|&&g| g < 1.0).count() as u64
}

/// The limiter's gain per frame for the frame peaks `tp` (of the buffer before the limiter) and
/// the output's true peak in dB, which `measure` gives for a gain (the true peak of the buffer
/// with that gain applied). `None` when no peak is over the ceiling: the buffer is untouched and
/// its true peak is the largest frame peak. Up to [`MAX_PASSES`] passes, each tightened by the
/// previous one's overshoot.
pub(crate) fn limiter_gain(
    tp: &[f64],
    sample_rate: u32,
    ceiling_db: f64,
    measure: impl Fn(&[f64]) -> f64,
) -> (Option<Vec<f64>>, f64) {
    let ceiling = 10f64.powf(ceiling_db / 20.0);
    if !tp.iter().any(|&p| p > ceiling) {
        return (None, 20.0 * peak_of_frames(tp).log10());
    }
    let mut g = vec![1.0f64; tp.len()];
    let mut pass_ceiling_db = ceiling_db;
    let mut pass = 1;
    loop {
        gains(tp, sample_rate, 10f64.powf(pass_ceiling_db / 20.0), &mut g);
        let got = measure(&g);
        let over = got.partial_cmp(&ceiling_db) == Some(std::cmp::Ordering::Greater);
        if !over || pass >= MAX_PASSES {
            return (Some(g), got);
        }
        pass_ceiling_db -= got - ceiling_db + TIGHTEN_MARGIN_DB;
        pass += 1;
    }
}

/// The gain per frame for frame peaks `tp` and a linear `ceiling`, into `out` (`tp.len()` long).
fn gains(tp: &[f64], sample_rate: u32, ceiling: f64, out: &mut [f64]) {
    let n = tp.len();
    // The required gain per frame (`ceiling / p` over the ceiling, else 1), spread over the
    // filter support: the minimum over i - SPREAD ..= i + SPREAD. Division is correctly rounded
    // and so monotone in `p`: the smallest `ceiling / p` of a window is `ceiling / max p`.
    for (i, slot) in out.iter_mut().enumerate() {
        let lo = i.saturating_sub(SPREAD);
        let hi = (i + SPREAD + 1).min(n);
        let peak = tp
            .get(lo..hi)
            .unwrap_or(&[])
            .iter()
            .fold(f64::NEG_INFINITY, |m, &p| m.max(p));
        *slot = if peak > ceiling { ceiling / peak } else { 1.0 };
    }
    // Instant attack over the lookahead: the gain falls linearly from 1 at L frames before a
    // peak to the required value at the peak. In place: frame `j`'s required gain is read before
    // any write to slot `j`, and the slots `i < j` already hold the attack of the frames before
    // `j` (a slot whose own required gain was 1 starts at 1, as the attack does).
    let l = lookahead_frames(sample_rate);
    for j in 0..n {
        let Some(&r) = out.get(j) else { break };
        if r >= 1.0 {
            continue;
        }
        let start = j.saturating_sub(l);
        for (k, slot) in out.iter_mut().enumerate().take(j + 1).skip(start) {
            // i - (j - L), with j - L possibly negative.
            let t = (k + l - j) as f64 / l as f64;
            let v = 1.0 - (1.0 - r) * t;
            // Slot `j` itself starts the attack at 1.
            let current = if k == j { 1.0 } else { *slot };
            *slot = if v < current { v } else { current };
        }
    }
    // Release.
    let decay = (-1.0 / (RELEASE_SEC * f64::from(sample_rate))).exp();
    let mut prev = 1.0f64;
    for g in out.iter_mut() {
        let s = g.min(1.0 - (1.0 - prev) * decay);
        *g = s;
        prev = s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::true_peak::{frame_peaks, true_peak_db, true_peak_db_by, true_peak_linear};

    const SR: u32 = 44_100;

    fn sine(hz: f64, amp: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(SR)).sin()) as f32
            })
            .collect()
    }

    /// What the limiter did.
    #[derive(Debug, PartialEq)]
    struct Limited {
        /// Frames attenuated (gain below 1).
        touched: u64,
        /// The output's true peak in dBTP, as `limiter_gain` measured it.
        true_peak_db: f64,
    }

    /// The limiter as `Prepared::level` runs it (with no static gain) on `x`: `limiter_gain` over
    /// the frame peaks, measured and then applied with `apply_one`.
    fn limited(x: &[Vec<f32>], ceiling_db: f64) -> (Vec<Vec<f32>>, Limited) {
        let tp = frame_peaks(x);
        let (gain, true_peak_db) = limiter_gain(&tp, SR, ceiling_db, |g| {
            true_peak_db_by(x, |i, v| g.get(i).map_or(v, |&g| apply_one(v, g)))
        });
        let Some(gain) = gain else {
            let l = Limited {
                touched: 0,
                true_peak_db,
            };
            return (x.to_vec(), l);
        };
        let out = x
            .iter()
            .map(|ch| {
                ch.iter()
                    .zip(&gain)
                    .map(|(&v, &g)| apply_one(v, g))
                    .collect()
            })
            .collect();
        let l = Limited {
            touched: touched(&gain),
            true_peak_db,
        };
        (out, l)
    }

    fn gain_of(tp: &[f64], ceiling: f64) -> Vec<f64> {
        let mut g = vec![0.0; tp.len()];
        gains(tp, SR, ceiling, &mut g);
        g
    }

    /// The gain as first written: separate arrays for the required gain, its spread and the
    /// attack, each a fold in frame order.
    fn reference_gains(tp: &[f64], sample_rate: u32, ceiling: f64) -> Vec<f64> {
        let n = tp.len();
        let req: Vec<f64> = tp
            .iter()
            .map(|&p| if p > ceiling { ceiling / p } else { 1.0 })
            .collect();
        let req2: Vec<f64> = (0..n)
            .map(|i| {
                let lo = i.saturating_sub(SPREAD);
                let hi = (i + SPREAD + 1).min(n);
                req[lo..hi].iter().fold(1.0f64, |m, &r| m.min(r))
            })
            .collect();
        let l = lookahead_frames(sample_rate);
        let mut m = vec![1.0f64; n];
        for (j, &r) in req2.iter().enumerate() {
            if r >= 1.0 {
                continue;
            }
            let start = j.saturating_sub(l);
            for (i, slot) in m.iter_mut().enumerate().take(j + 1).skip(start) {
                let t = (i + l - j) as f64 / l as f64;
                let v = 1.0 - (1.0 - r) * t;
                if v < *slot {
                    *slot = v;
                }
            }
        }
        let decay = (-1.0 / (RELEASE_SEC * f64::from(sample_rate))).exp();
        let mut prev = 1.0f64;
        for g in m.iter_mut() {
            let s = g.min(1.0 - (1.0 - prev) * decay);
            *g = s;
            prev = s;
        }
        m
    }

    #[test]
    fn level_limiter_gains_equal_the_reference_bit_for_bit() {
        let mut state: u32 = 99;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f64::from(state) / f64::from(u32::MAX)
        };
        let noise: Vec<f32> = (0..30_000)
            .map(|_| ((next() * 2.0 - 1.0) * 1.6) as f32)
            .collect();
        let bursts: Vec<f32> = (0..30_000)
            .map(|i| {
                let amp = if (i / 997) % 3 == 0 { 1.9 } else { 0.4 };
                (amp * (i as f64 * 0.07).sin()) as f32
            })
            .collect();
        let edges = {
            let mut v = vec![0.0f32; 300];
            v[0] = 3.0;
            v[150] = 2.0;
            v[299] = -3.0;
            v
        };
        for (name, x) in [("noise", noise), ("bursts", bursts), ("edges", edges)] {
            let tp = frame_peaks(std::slice::from_ref(&x));
            for (sr, ceiling_db) in [(SR, -1.0), (SR, -6.0), (48_000, -1.0), (8_000, -3.0)] {
                let c = 10f64.powf(ceiling_db / 20.0);
                let want = reference_gains(&tp, sr, c);
                let mut got = vec![0.0; tp.len()];
                gains(&tp, sr, c, &mut got);
                let want: Vec<u64> = want.iter().map(|v| v.to_bits()).collect();
                let got: Vec<u64> = got.iter().map(|v| v.to_bits()).collect();
                assert_eq!(got, want, "{name} {sr} {ceiling_db}");
            }
        }
    }

    fn bits(c: &[Vec<f32>]) -> Vec<Vec<u32>> {
        c.iter()
            .map(|ch| ch.iter().map(|x| x.to_bits()).collect())
            .collect()
    }

    #[test]
    fn level_limiter_parameters() {
        assert_eq!(lookahead_frames(44_100), 66);
        assert_eq!(lookahead_frames(48_000), 72);
        assert_eq!(lookahead_frames(1), 1);
        assert_eq!(SPREAD, 6);
    }

    #[test]
    fn level_limiter_is_a_bit_identical_no_op_under_the_ceiling() {
        let x = vec![sine(997.0, 0.5, 44_100), sine(3000.0, 0.8, 44_100)];
        // 0.8 peak is -1.94 dBTP at most (fs/14.7: little intersample overshoot).
        let (out, l) = limited(&x, -1.0);
        assert_eq!(l.touched, 0);
        assert_eq!(l.true_peak_db, true_peak_db(&x));
        assert_eq!(bits(&out), bits(&x));
        // Exactly at the ceiling is not over.
        let tp = true_peak_db(&x);
        let (out, l) = limited(&x, tp);
        assert_eq!(l.touched, 0);
        assert_eq!(bits(&out), bits(&x));
        // Empty and silent input.
        assert_eq!(
            limited(&[], -1.0).1,
            Limited {
                touched: 0,
                true_peak_db: f64::NEG_INFINITY
            }
        );
        let z = vec![vec![0.0f32; 100]; 2];
        assert_eq!(limited(&z, -1.0).0, z);
        // Signed zeros and subnormals keep their bits.
        let odd = vec![vec![-0.0f32, f32::MIN_POSITIVE / 2.0, 0.25, -0.0]];
        assert_eq!(bits(&limited(&odd, -1.0).0), bits(&odd));
    }

    #[test]
    fn level_limiter_never_exceeds_the_ceiling() {
        let cases: Vec<(&str, Vec<Vec<f32>>, f64)> = vec![
            ("hot sine", vec![sine(1000.0, 1.5, 22_050); 2], -1.0),
            (
                "quarter-rate 45 deg",
                vec![
                    (0..22_050)
                        .map(|i| {
                            (1.4 * (std::f64::consts::FRAC_PI_2 * i as f64
                                + std::f64::consts::FRAC_PI_4)
                                .sin()) as f32
                        })
                        .collect(),
                ],
                -1.0,
            ),
            (
                "alternating full scale",
                vec![
                    (0..5000)
                        .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
                        .collect();
                    2
                ],
                -3.0,
            ),
            (
                "single impulse",
                vec![{
                    let mut v = vec![0.0f32; 2000];
                    v[1000] = 4.0;
                    v
                }],
                -1.0,
            ),
            (
                "impulse at the edges",
                vec![{
                    let mut v = vec![0.0f32; 300];
                    v[0] = 3.0;
                    v[299] = -3.0;
                    v
                }],
                -1.0,
            ),
            ("one frame", vec![vec![2.0f32]], -1.0),
            (
                "deep ceiling",
                vec![sine(5000.0, 1.0, 10_000), sine(7000.0, 0.9, 10_000)],
                -20.0,
            ),
            (
                "noise",
                vec![{
                    let mut s: u32 = 1;
                    (0..20_000)
                        .map(|_| {
                            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                            (f64::from(s) / 2_147_483_648.0 - 1.0) as f32 * 1.8
                        })
                        .collect()
                }],
                -1.0,
            ),
        ];
        for (name, x, ceiling) in cases {
            let (out, l) = limited(&x, ceiling);
            let got = true_peak_db(&out);
            assert_eq!(l.true_peak_db, got, "{name}: the reported peak");
            assert!(got <= ceiling, "{name}: {got} > {ceiling}");
            assert!(l.touched > 0, "{name}");
            assert_eq!(out.len(), x.len(), "{name}");
            for (o, i) in out.iter().zip(&x) {
                assert_eq!(o.len(), i.len(), "{name}: length");
                assert!(o.iter().all(|v| v.is_finite()), "{name}: finite");
            }
            assert!(l.touched <= x[0].len() as u64, "{name}");
        }
    }

    #[test]
    fn level_limiter_gain_starts_a_lookahead_before_the_peak() {
        let mut x = vec![0.1f32; 2000];
        x[1000] = 2.0;
        let tp = frame_peaks(&[x.clone()]);
        let g = gain_of(&tp, 1.0);
        let l = lookahead_frames(SR);
        // Untouched before the lookahead window of the first spread frame.
        let first = 1000 - SPREAD - l;
        assert!(
            g[..=first].iter().all(|&v| v == 1.0),
            "before the lookahead"
        );
        assert!(g[first + 1] < 1.0);
        // At the peak the gain is at most the required gain.
        assert!(
            g[1000] * tp[1000] <= 1.0 + 1e-12,
            "{} {}",
            g[1000],
            tp[1000]
        );
        // Monotone down to the spread window, then flat at its minimum through it.
        for i in first..1000 - SPREAD {
            assert!(g[i + 1] <= g[i], "attack at {i}");
        }
    }

    #[test]
    fn level_limiter_release_time_constant() {
        // A 0.5 sine with a burst at 2.0 in the middle: after the burst the gain recovers with a
        // 50 ms time constant.
        let frames = 44_100;
        let (b0, b1) = (20_000usize, 22_000usize);
        let x: Vec<f32> = (0..frames)
            .map(|i| {
                let amp = if (b0..b1).contains(&i) { 2.0 } else { 0.5 };
                (amp * (2.0 * std::f64::consts::PI * 441.0 * i as f64 / f64::from(SR)).sin()) as f32
            })
            .collect();
        let tp = frame_peaks(std::slice::from_ref(&x));
        let g = gain_of(&tp, 10f64.powf(-1.0 / 20.0));
        let g_min = g.iter().fold(1.0f64, |m, &v| m.min(v));
        let at = b1 + (RELEASE_SEC * f64::from(SR)).round() as usize;
        let want = 1.0 - (1.0 - g_min) * (-1.0f64).exp();
        assert!(((g[at] - want) / want).abs() < 0.05, "{} vs {want}", g[at]);
        // The limited output stays under the ceiling and keeps its length.
        let (out, _) = limited(std::slice::from_ref(&x), -1.0);
        assert_eq!(out[0].len(), frames);
        assert!(true_peak_db(&out) <= -1.0);
        // Back to unity well after the burst.
        assert!(g[frames - 1] > 0.999, "{}", g[frames - 1]);
    }

    #[test]
    fn level_limiter_mono_and_unequal_channels() {
        let mono = vec![sine(2000.0, 1.3, 8000)];
        let (out, _) = limited(&mono, -1.0);
        assert!(true_peak_db(&out) <= -1.0);
        assert_eq!(out[0].len(), 8000);
        // Channels of different lengths keep their own lengths.
        let x = vec![sine(2000.0, 1.3, 8000), sine(2000.0, 1.3, 5000)];
        let (out, _) = limited(&x, -1.0);
        assert_eq!(out[0].len(), 8000);
        assert_eq!(out[1].len(), 5000);
        assert!(true_peak_linear(&out) <= 10f64.powf(-1.0 / 20.0));
    }
}
