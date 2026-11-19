//! The onset strength function and its peaks.
//!
//! An STFT of `ONSET_WIN` samples (Hann) at the pitch track's hop, frame `i` centred at
//! `i * ONSET_HOP` like the pitch frames, so strength `i` and pitch frame `i` describe the same
//! moment. Per frame:
//!
//! - the spectral flux: the half-wave rectified rise of `log(1 + ONSET_LOG_GAIN * mag)` summed
//!   over the bins in `ONSET_BAND_HZ` (the frame before the first is silence), each bin compared
//!   with the previous frame's maximum over `ONSET_MAX_FILTER_BINS` bins either side (the
//!   SuperFlux maximum filter of Böck and Widmer, DAFx 2013), so vibrato adds no flux.
//!   The flux is scaled by the window's energy `ONSET_DECAY_LOOK_MS` later over its energy as
//!   long before, capped at 1, so the spectral splatter of a fast release is not an attack;
//! - the RMS rise: the half-wave rectified rise of the window's RMS.
//!
//! Each is divided by its `ONSET_NORM_PERCENTILE` (at least `ONSET_NORM_FLOOR` of its maximum)
//! and clamped to `ONSET_NORM_CLAMP`; the strength is `flux + ONSET_RMS_WEIGHT * rise`, scaled to
//! 0..=1 by its maximum. A peak is a frame at or above the adaptive threshold (the mean strength
//! within `ONSET_THRESH_WINDOW_MS` each side plus `ONSET_THRESH_DELTA`) that is the maximum
//! within `ONSET_MIN_DISTANCE_MS` each side (ties go to the earliest frame).

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;
use serde::{Deserialize, Serialize};

use crate::consts::{
    ANALYSIS_RATE, FRAME, ONSET_BAND_HZ, ONSET_DECAY_LOOK_MS, ONSET_HOP, ONSET_LOG_GAIN,
    ONSET_MAX_FILTER_BINS, ONSET_MIN_DISTANCE_MS, ONSET_NORM_CLAMP, ONSET_NORM_FLOOR,
    ONSET_NORM_PERCENTILE, ONSET_RMS_WEIGHT, ONSET_THRESH_DELTA, ONSET_THRESH_WINDOW_MS, ONSET_WIN,
    REARTICULATION_WIN,
};
use crate::util::ms_to_frames;

/// The onset strength of a signal, one value per pitch frame, and its peaks.
#[derive(Debug, Clone, PartialEq)]
pub struct OnsetTrack {
    /// Seconds between frames: the pitch track's hop.
    pub hop_seconds: f32,
    /// Strength per frame, 0..=1; frame `i` is centred at `i * hop_seconds`.
    pub strength: Vec<f32>,
    /// The RMS of the `REARTICULATION_WIN` samples centred on each frame, as long as
    /// `strength`; empty when not measured (a hand-built track).
    pub rms: Vec<f32>,
    pub peaks: Vec<Onset>,
}

/// A peak of the onset strength.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Onset {
    /// Seconds from the start of the analysed samples.
    pub seconds: f64,
    /// The strength at the peak, 0..=1.
    pub strength: f32,
}

/// The onset strength of f32 samples, non-finite ones as silence.
#[cfg(test)]
pub fn onset_strength(samples_16k: &[f32]) -> OnsetTrack {
    onset_strength_of_signal(&crate::util::finite_f64(samples_16k))
}

/// The onset strength of 16 kHz mono samples, converted by `util::finite_f64` (the pipeline
/// shares the conversion with the pitch tracker). Input shorter than a pitch frame (`FRAME`)
/// gives an empty track, as the pitch tracker does; otherwise there are as many values as pitch
/// frames.
pub(crate) fn onset_strength_of_signal(signal: &[f64]) -> OnsetTrack {
    let hop_seconds = ONSET_HOP as f32 / ANALYSIS_RATE as f32;
    if signal.len() < FRAME {
        return OnsetTrack {
            hop_seconds,
            strength: Vec::new(),
            rms: Vec::new(),
            peaks: Vec::new(),
        };
    }
    let n_frames = 1 + signal.len() / ONSET_HOP;
    let (flux, rise, rms) = flux_and_rise(signal, n_frames);
    let flux = normalize(&flux);
    let rise = normalize(&rise);
    let sum: Vec<f64> = flux
        .iter()
        .zip(&rise)
        .map(|(f, r)| f + f64::from(ONSET_RMS_WEIGHT) * r)
        .collect();
    let max = sum.iter().copied().fold(0.0f64, f64::max);
    let strength: Vec<f32> = sum
        .iter()
        .map(|&v| if max > 0.0 { (v / max) as f32 } else { 0.0 })
        .collect();
    let peaks = pick_onsets(&strength, hop_seconds);
    OnsetTrack {
        hop_seconds,
        strength,
        rms,
        peaks,
    }
}

/// The peaks of a strength function sampled every `hop_seconds`: frames at or above the adaptive
/// threshold that are the maximum within `ONSET_MIN_DISTANCE_MS` each side (ties go to the
/// earliest), so two peaks are always more than that far apart.
pub fn pick_onsets(strength: &[f32], hop_seconds: f32) -> Vec<Onset> {
    let hop = f64::from(hop_seconds);
    if !(hop.is_finite() && hop > 0.0) || strength.is_empty() {
        return Vec::new();
    }
    let s: Vec<f64> = strength
        .iter()
        .map(|&v| if v.is_finite() { f64::from(v) } else { 0.0 })
        .collect();
    let n = s.len();
    let mut prefix = Vec::with_capacity(n + 1);
    prefix.push(0.0f64);
    for v in &s {
        prefix.push(prefix[prefix.len() - 1] + v);
    }
    let w = ms_to_frames(ONSET_THRESH_WINDOW_MS, hop);
    let d = ms_to_frames(ONSET_MIN_DISTANCE_MS, hop);
    let mut peaks = Vec::new();
    for i in 0..n {
        if s[i] <= 0.0 {
            continue;
        }
        let (lo, hi) = (i.saturating_sub(w), (i + w + 1).min(n));
        let mean = (prefix[hi] - prefix[lo]) / (hi - lo) as f64;
        if s[i] < mean + f64::from(ONSET_THRESH_DELTA) {
            continue;
        }
        let (lo, hi) = (i.saturating_sub(d), (i + d + 1).min(n));
        let earlier_lower = s[lo..i].iter().all(|&v| v < s[i]);
        let later_not_higher = s[i + 1..hi].iter().all(|&v| v <= s[i]);
        if earlier_lower && later_not_higher {
            peaks.push(Onset {
                seconds: i as f64 * hop,
                strength: s[i] as f32,
            });
        }
    }
    peaks
}

/// The raw spectral flux (decay-gated), RMS rise and RMS per frame.
fn flux_and_rise(signal: &[f64], n_frames: usize) -> (Vec<f64>, Vec<f64>, Vec<f32>) {
    let mut planner = RealFftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(ONSET_WIN);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();
    let mut scratch = vec![Complex::new(0.0, 0.0); fft.get_scratch_len()];
    let window: Vec<f64> = (0..ONSET_WIN)
        .map(|j| 0.5 - 0.5 * (std::f64::consts::TAU * j as f64 / ONSET_WIN as f64).cos())
        .collect();
    // a full-scale sine has magnitude 1
    let norm = 2.0 / window.iter().sum::<f64>();
    let bin_hz = f64::from(ANALYSIS_RATE) / ONSET_WIN as f64;
    let lo = (f64::from(ONSET_BAND_HZ.0) / bin_hz).ceil() as usize;
    let hi = ((f64::from(ONSET_BAND_HZ.1) / bin_hz).floor() as usize).min(ONSET_WIN / 2);
    let band = lo.min(hi)..=hi;
    let look = ms_to_frames(
        ONSET_DECAY_LOOK_MS,
        f64::from(ONSET_HOP as u32) / f64::from(ANALYSIS_RATE),
    );
    let len = signal.len() as isize;
    let start_of = |t: usize| (t * ONSET_HOP) as isize - (ONSET_WIN / 2) as isize;
    let sample = |i: isize| {
        if (0..len).contains(&i) {
            signal[i as usize]
        } else {
            0.0
        }
    };
    // mean square of each frame's (unwindowed) samples, `look` frames past the end too
    let energy: Vec<f64> = (0..n_frames + look)
        .map(|t| {
            let start = start_of(t);
            (0..ONSET_WIN as isize)
                .map(|j| sample(start + j).powi(2))
                .sum::<f64>()
                / ONSET_WIN as f64
        })
        .collect();
    let bins = spectrum.len();
    let mut prev_log = vec![0.0f64; bins];
    let mut cur_log = vec![0.0f64; bins];
    let mut flux = Vec::with_capacity(n_frames);
    let mut rise = Vec::with_capacity(n_frames);

    for t in 0..n_frames {
        let start = start_of(t);
        for (j, x) in input.iter_mut().enumerate() {
            *x = sample(start + j as isize) * window[j];
        }
        let prev_rms = t.checked_sub(1).map_or(0.0, |p| energy[p].sqrt());
        rise.push((energy[t].sqrt() - prev_rms).max(0.0));

        let mut f = 0.0;
        if fft
            .process_with_scratch(&mut input, &mut spectrum, &mut scratch)
            .is_ok()
        {
            for (l, x) in cur_log.iter_mut().zip(&spectrum) {
                *l = (1.0 + ONSET_LOG_GAIN * x.norm() * norm).ln();
            }
            for k in band.clone() {
                // the previous frame's maximum over the neighbouring bins: vibrato moves a
                // harmonic less than that per hop, so it adds no flux
                let lo = k.saturating_sub(ONSET_MAX_FILTER_BINS);
                let hi = (k + ONSET_MAX_FILTER_BINS + 1).min(bins);
                let reference = prev_log[lo..hi].iter().copied().fold(0.0f64, f64::max);
                f += (cur_log[k] - reference).max(0.0);
            }
            std::mem::swap(&mut prev_log, &mut cur_log);
        }
        // a decaying sound (an offset's spectral splatter) is not an attack
        let before = t.checked_sub(look).map_or(0.0, |b| energy[b]);
        let after = energy[t + look];
        if before > 0.0 {
            f *= (after / before).min(1.0);
        }
        flux.push(f);
    }
    let rms = (0..n_frames)
        .map(|t| {
            let start = (t * ONSET_HOP) as isize - (REARTICULATION_WIN / 2) as isize;
            let sum: f64 = (0..REARTICULATION_WIN as isize)
                .map(|j| sample(start + j).powi(2))
                .sum();
            (sum / REARTICULATION_WIN as f64).sqrt() as f32
        })
        .collect();
    (flux, rise, rms)
}

/// `values` divided by their percentile (at least the floor fraction of their maximum), clamped.
/// All zeros stay zeros.
fn normalize(values: &[f64]) -> Vec<f64> {
    let max = values.iter().copied().fold(0.0f64, f64::max);
    if max <= 0.0 {
        return vec![0.0; values.len()];
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = (ONSET_NORM_PERCENTILE * (sorted.len() - 1) as f64).round() as usize;
    let divisor = sorted[rank.min(sorted.len() - 1)].max(ONSET_NORM_FLOOR * max);
    values
        .iter()
        .map(|v| (v / divisor).min(ONSET_NORM_CLAMP))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::synth::{Case, SplitMix64, SynthSettings};
    use crate::test_support::{audio, hop, track, voice};

    fn peak_times(track: &OnsetTrack) -> Vec<f64> {
        track.peaks.iter().map(|p| p.seconds).collect()
    }

    /// The peak nearest `t` and its distance, in seconds.
    fn nearest(track: &OnsetTrack, t: f64) -> Option<(f64, f64)> {
        track
            .peaks
            .iter()
            .map(|p| (p.seconds, (p.seconds - t).abs()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    #[test]
    fn strength_peaks_at_known_attacks() {
        let notes = [
            (0.30, 0.70, 60),
            (0.70, 1.00, 64),
            (1.00, 1.60, 67),
            (1.90, 2.30, 72),
            (2.30, 2.55, 71),
        ];
        let o = onset_strength(&audio(&notes, voice()));
        for (start, _, midi) in notes {
            let (at, err) = nearest(&o, start).expect("some peak");
            assert!(
                err <= 0.02,
                "note {midi} at {start}: nearest peak {at} ({:.0} ms off) in {:?}",
                err * 1000.0,
                peak_times(&o)
            );
        }
        assert_eq!(
            o.peaks.len(),
            notes.len(),
            "one peak per note: {:?}",
            o.peaks
        );
    }

    #[test]
    fn rms_follows_the_short_window() {
        // a 0.5-amplitude 200 Hz sine from 0.5 s: RMS 0.354 once the window is inside it
        let rate = ANALYSIS_RATE as f64;
        let samples: Vec<f32> = (0..16_000)
            .map(|i| {
                let t = i as f64 / rate;
                if t < 0.5 {
                    0.0
                } else {
                    (0.5 * (std::f64::consts::TAU * 200.0 * t).sin()) as f32
                }
            })
            .collect();
        let o = onset_strength(&samples);
        assert_eq!(o.rms.len(), o.strength.len());
        let h = f64::from(hop());
        let at = |t: f64| o.rms[(t / h).round() as usize];
        assert_eq!(at(0.3), 0.0);
        assert!((at(0.8) - 0.3536).abs() < 0.005, "{}", at(0.8));
        // the window is 20 ms: 10 ms after the start it is half full
        let half = at(0.5);
        assert!((half - 0.25).abs() < 0.02, "{half}");
        assert!(at(0.52) > 0.35);
    }

    #[test]
    fn repeated_same_pitch_notes_give_peaks() {
        let notes = [
            (0.30, 0.55, 67),
            (0.55, 0.80, 67),
            (0.80, 1.05, 67),
            (1.05, 1.50, 67),
        ];
        let o = onset_strength(&audio(&notes, voice()));
        for (start, _, _) in notes {
            let (at, err) = nearest(&o, start).expect("some peak");
            assert!(
                err <= 0.02,
                "repeat at {start}: nearest peak {at} in {:?}",
                peak_times(&o)
            );
        }
        assert_eq!(o.peaks.len(), 4, "{:?}", o.peaks);
    }

    #[test]
    fn steady_tone_has_no_peaks_after_start() {
        let o = onset_strength(&audio(&[(0.25, 2.5, 69)], voice()));
        assert_eq!(o.peaks.len(), 1, "{:?}", o.peaks);
        assert!((o.peaks[0].seconds - 0.25).abs() <= 0.02, "{:?}", o.peaks);
        // with vibrato the flux never rises to a peak either
        let settings = SynthSettings {
            vibrato_cents: 50.0,
            ..voice()
        };
        let o = onset_strength(&audio(&[(0.25, 2.5, 69)], settings));
        let late: Vec<_> = o.peaks.iter().filter(|p| p.seconds > 0.35).collect();
        assert!(late.is_empty(), "vibrato peaks {late:?}");
    }

    #[test]
    fn silence_has_none() {
        let o = onset_strength(&vec![0.0; 32_000]);
        assert_eq!(o.strength.len(), 1 + 32_000 / ONSET_HOP);
        assert!(o.strength.iter().all(|&s| s == 0.0), "zero strength");
        assert!(o.peaks.is_empty());
        // non-finite samples are silence
        let o = onset_strength(&vec![f32::NAN; 4_000]);
        assert!(o.strength.iter().all(|&s| s == 0.0) && o.peaks.is_empty());
    }

    #[test]
    fn peaks_respect_min_distance() {
        // full-scale noise has flux everywhere: peaks are still spaced
        let mut rng = SplitMix64::new(3);
        let noise: Vec<f32> = (0..48_000).map(|_| rng.uniform(-1.0, 1.0) as f32).collect();
        let o = onset_strength(&noise);
        let min_gap = f64::from(ONSET_MIN_DISTANCE_MS) / 1000.0;
        for pair in o.peaks.windows(2) {
            assert!(
                pair[1].seconds - pair[0].seconds > min_gap,
                "noise peaks {pair:?}"
            );
        }
        // two equal impulses 30 ms apart give one peak, at the earlier
        let mut s = vec![0.0f32; 100];
        s[40] = 1.0;
        s[43] = 1.0;
        let peaks = pick_onsets(&s, hop());
        assert_eq!(peaks.len(), 1, "{peaks:?}");
        assert!((peaks[0].seconds - 0.40).abs() < 1e-6);
        // 60 ms apart gives both
        s[43] = 0.0;
        s[46] = 0.8;
        let peaks = pick_onsets(&s, hop());
        assert_eq!(peaks.len(), 2, "{peaks:?}");
        // a plateau gives one peak at its start
        let mut s = vec![0.0f32; 100];
        s[20..40].fill(1.0);
        let peaks = pick_onsets(&s, hop());
        assert_eq!(peaks.len(), 1, "{peaks:?}");
        assert!((peaks[0].seconds - 0.20).abs() < 1e-6);
    }

    #[test]
    fn strength_is_0_to_1() {
        let mut rng = SplitMix64::new(9);
        let noise: Vec<f32> = (0..20_000).map(|_| rng.uniform(-1.0, 1.0) as f32).collect();
        let case = Case::generate(1000, &SynthSettings::default());
        let melody = case.render(ANALYSIS_RATE);
        for (label, samples) in [("noise", noise), ("melody", melody)] {
            let o = onset_strength(&samples);
            assert!(
                o.strength.iter().all(|s| (0.0..=1.0).contains(s)),
                "{label}: out of range"
            );
            let max = o.strength.iter().copied().fold(0.0f32, f32::max);
            assert_eq!(max, 1.0, "{label}: scaled to its maximum");
            assert!(
                o.peaks.iter().all(|p| (0.0..=1.0).contains(&p.strength)),
                "{label}: peak strength"
            );
        }
    }

    #[test]
    fn empty_and_short_input_are_empty() {
        for len in [0, 1, 160, FRAME - 1] {
            let o = onset_strength(&vec![0.5; len]);
            assert!(o.strength.is_empty() && o.peaks.is_empty(), "len {len}");
            assert_eq!(o.hop_seconds, hop());
        }
        assert!(pick_onsets(&[], hop()).is_empty());
        assert!(pick_onsets(&[1.0, 0.0], 0.0).is_empty(), "zero hop");
        assert!(pick_onsets(&[0.0; 50], hop()).is_empty(), "all zero");
    }

    #[test]
    fn strength_length_matches_pitch_frames() {
        for len in [FRAME, FRAME + 1, 1_600, 16_159, 16_160, 33_333] {
            let samples: Vec<f32> = (0..len).map(|i| (i as f32 * 0.1).sin() * 0.3).collect();
            let o = onset_strength(&samples);
            let p = track(&samples);
            assert_eq!(o.strength.len(), p.frames.len(), "len {len}");
            assert_eq!(o.hop_seconds, p.hop_seconds, "len {len}");
        }
    }

    #[test]
    fn onset_serializes_camel_case() {
        let v = serde_json::to_value(Onset {
            seconds: 1.5,
            strength: 0.5,
        })
        .unwrap();
        assert_eq!(v, serde_json::json!({"seconds": 1.5, "strength": 0.5}));
    }
}
