//! The global tuning offset of a take.
//!
//! The offset is the circular mean of each stable frame's deviation from 12-TET (A4 = 440 Hz),
//! weighted by the frame's voiced probability. Deviations live on a circle of 100 cents, so a
//! singer 49 cents sharp and one 49 cents flat average to 50, not 0. Only stable frames count:
//! the first `TUNING_SKIP_MS` of each voiced run (the attack) is skipped, and so is any frame
//! whose smoothed MIDI pitch moves more than `TUNING_STABLE_ST` over `TUNING_STABLE_MS`
//! (glides). Positive means sharp.
//!
//! The smoothed pitch is a centred median over `TUNING_CENTRE_MS` (a vibrato cycle), clipped to the
//! run, and a frame's deviation is that smoothed pitch's. A vibrato of 25 cents or more spreads
//! the raw frames around the whole circle (its stable extremes at +-50 cents are one point on
//! it), so the raw frames cannot be averaged; the cycle median is the pitch the singer centres on,
//! and unlike a mean it ignores an attack glide inside its window.

use std::f64::consts::TAU;

use crate::consts::{TUNING_CENTRE_MS, TUNING_SKIP_MS, TUNING_STABLE_MS, TUNING_STABLE_ST};
use crate::pitch::PitchTrack;
use crate::util::{clamp01, frame_midi, hop_of, median_filter, ms_to_frames, voiced_runs};

/// The tuning offset of `track` in cents, -50..=50 (positive is sharp); 0.0 when no frame is
/// stable.
pub fn estimate_tuning(track: &PitchTrack) -> f32 {
    let Some(hop) = hop_of(track) else {
        return 0.0;
    };
    let skip = ms_to_frames(TUNING_SKIP_MS, hop);
    let (mut c, mut s) = (0.0f64, 0.0f64);
    for run in voiced_runs(track) {
        let midi: Vec<f32> = run.clone().filter_map(|i| frame_midi(track, i)).collect();
        let centred = median_filter(&midi, ms_to_frames(TUNING_CENTRE_MS, hop) / 2);
        let stable = stable_mask(&centred, hop);
        for (k, i) in run.enumerate().skip(skip) {
            if !stable[k] {
                continue;
            }
            let dev = f64::from(centred[k] - centred[k].round());
            let w = f64::from(clamp01(track.frames[i].voiced_prob));
            c += w * (TAU * dev).cos();
            s += w * (TAU * dev).sin();
        }
    }
    if c.hypot(s) < 1e-9 {
        return 0.0;
    }
    ((s.atan2(c) / TAU * 100.0) as f32).clamp(-50.0, 50.0)
}

/// For each frame of a smoothed run, whether the pitch moves at most `TUNING_STABLE_ST` within
/// the `TUNING_STABLE_MS` window centred on it (clipped at the run's ends).
fn stable_mask(smoothed: &[f32], hop: f64) -> Vec<bool> {
    let half = ms_to_frames(TUNING_STABLE_MS, hop) / 2;
    (0..smoothed.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(smoothed.len());
            let window = &smoothed[lo..hi];
            let max = window.iter().copied().fold(f32::MIN, f32::max);
            let min = window.iter().copied().fold(f32::MAX, f32::min);
            max - min <= TUNING_STABLE_ST
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::synth::SynthSettings;
    use crate::test_support::{audio, run, sine, track, track_of, voice};

    /// The tuning of a voice holding `midi` notes for 0.5 s each, detuned by `cents`.
    fn tuning_of(notes: &[i32], cents: f64, settings: SynthSettings) -> f32 {
        let notes: Vec<(f64, f64, i32)> = notes
            .iter()
            .enumerate()
            .map(|(k, &m)| (0.2 + 0.6 * k as f64, 0.7 + 0.6 * k as f64, m))
            .collect();
        let samples = audio(
            &notes,
            SynthSettings {
                detune_cents: cents,
                ..settings
            },
        );
        estimate_tuning(&track(&samples))
    }

    #[test]
    fn zero_for_a440_tone() {
        let cents = tuning_of(&[69], 0.0, sine());
        assert!(cents.abs() <= 1.0, "a440 sine: {cents} cents");
        let cents = tuning_of(&[69, 72, 76], 0.0, voice());
        assert!(cents.abs() <= 1.0, "in-tune voice: {cents} cents");
    }

    #[test]
    fn flat_30_cents_reports_minus_30() {
        let cents = tuning_of(&[60, 64, 67, 72], -30.0, voice());
        assert!((cents + 30.0).abs() <= 1.0, "30 cents flat: {cents} cents");
    }

    #[test]
    fn sharp_sign_is_positive() {
        for want in [10.0, 25.0, 40.0] {
            let cents = tuning_of(&[62, 65, 69], want, voice());
            assert!(
                (f64::from(cents) - want).abs() <= 1.0,
                "{want} cents sharp: {cents} cents"
            );
        }
    }

    #[test]
    fn glides_and_attacks_do_not_bias() {
        // each note starts 100 cents flat and glides up over 60 ms; the attacks are skipped
        let settings = SynthSettings {
            attack_glide_ms: 60.0,
            ..voice()
        };
        let cents = tuning_of(&[60, 62, 64, 65, 67], 0.0, settings);
        assert!(cents.abs() <= 1.5, "attack glides: {cents} cents");
        // portamento between adjacent notes: the glides are unstable frames
        let notes: Vec<(f64, f64, i32)> = [60, 64, 67, 64, 60]
            .iter()
            .enumerate()
            .map(|(k, &m)| (0.2 + 0.4 * k as f64, 0.6 + 0.4 * k as f64, m))
            .collect();
        let settings = SynthSettings {
            portamento_ms: 80.0,
            detune_cents: 20.0,
            ..voice()
        };
        let cents = estimate_tuning(&track(&audio(&notes, settings)));
        assert!((cents - 20.0).abs() <= 1.5, "portamento: {cents} cents");
    }

    #[test]
    fn vibrato_does_not_bias() {
        // a +-50 cents vibrato spans the whole circle; its cycle median does not
        for (cents, depth) in [(0.0, 50.0), (20.0, 50.0), (-15.0, 35.0)] {
            let settings = SynthSettings {
                vibrato_cents: depth,
                ..voice()
            };
            let got = tuning_of(&[60, 64, 67, 72], cents, settings);
            assert!(
                (f64::from(got) - cents).abs() <= 3.0,
                "{cents} cents with +-{depth} vibrato: {got} cents"
            );
        }
    }

    #[test]
    fn wraps_at_50_cents() {
        // 55 cents sharp of A is 45 cents flat of B-flat
        let cents = tuning_of(&[69], 55.0, sine());
        assert!((cents + 45.0).abs() <= 1.0, "55 sharp: {cents} cents");
        let cents = tuning_of(&[69], -55.0, sine());
        assert!((cents - 45.0).abs() <= 1.0, "55 flat: {cents} cents");
        // frames at +49 and -49 cents are 2 cents apart on the circle: the mean is 50, not 0
        let mut midi = run(Some(69.49), 60);
        midi.extend(run(None, 10));
        midi.extend(run(Some(68.51), 60));
        let cents = estimate_tuning(&track_of(&midi, 0.9));
        assert!((cents.abs() - 50.0).abs() <= 0.1, "+-49: {cents} cents");
    }

    #[test]
    fn no_voiced_frames_is_zero() {
        assert_eq!(estimate_tuning(&track_of(&[], 0.9)), 0.0, "empty track");
        assert_eq!(
            estimate_tuning(&track_of(&run(None, 100), 0.9)),
            0.0,
            "unvoiced track"
        );
        let silence = vec![0.0f32; 16_000];
        assert_eq!(estimate_tuning(&track(&silence)), 0.0, "silence");
        // a run no longer than the attack skip has no stable frame
        let mut midi = run(None, 10);
        midi.extend(run(Some(69.3), 3));
        midi.extend(run(None, 10));
        assert_eq!(estimate_tuning(&track_of(&midi, 0.9)), 0.0, "attack only");
        // a zero hop is not a track
        let mut t = track_of(&run(Some(69.3), 50), 0.9);
        t.hop_seconds = 0.0;
        assert_eq!(estimate_tuning(&t), 0.0, "zero hop");
    }

    #[test]
    fn weights_by_voiced_probability() {
        // 60 frames +20 cents at prob 0.9, 60 frames -20 cents at prob 0.1
        let mut t = track_of(&run(Some(69.2), 60), 0.9);
        let mut flat = track_of(&run(Some(68.8), 60), 0.1);
        t.frames.push(crate::pitch::PitchFrame {
            f0: None,
            voiced_prob: 0.0,
            rms: 0.0,
        });
        t.frames.append(&mut flat.frames);
        let cents = estimate_tuning(&t);
        // circular mean of +20 (weight 0.9 * 57) and -20 (weight 0.1 * 57)
        let (w1, w2) = (0.9f64, 0.1f64);
        let a = TAU * 0.2;
        let want = ((w1 - w2) * a.sin()).atan2((w1 + w2) * a.cos()) / TAU * 100.0;
        assert!(
            (f64::from(cents) - want).abs() < 0.05,
            "weighted: {cents} vs {want}"
        );
    }

    #[test]
    fn stable_mask_marks_still_pitch() {
        let mask = stable_mask(&[60.0, 60.0, 60.0, 60.5, 61.0, 61.0, 61.0, 61.0], 0.01);
        assert_eq!(
            mask,
            vec![true, false, false, false, false, false, true, true]
        );
    }
}
