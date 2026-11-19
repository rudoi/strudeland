//! Small helpers the stages share: pitch-track access, frame counts, medians and MIDI/Hz.

use std::ops::Range;

use crate::consts::MEDIAN_FRAMES;
use crate::pitch::PitchTrack;

/// `samples` as f64, a non-finite sample as 0 (silence).
pub(crate) fn finite_f64(samples: &[f32]) -> Vec<f64> {
    samples
        .iter()
        .map(|&s| if s.is_finite() { f64::from(s) } else { 0.0 })
        .collect()
}

/// The frequency in Hz of the fractional MIDI note `midi` (A4 = 440 Hz = 69).
#[cfg(any(test, feature = "eval"))]
pub(crate) fn midi_to_hz(midi: f64) -> f64 {
    440.0 * ((midi - 69.0) / 12.0).exp2()
}

/// The fractional MIDI note of `hz` (A4 = 440 Hz = 69).
#[cfg(test)]
pub(crate) fn hz_to_midi(hz: f64) -> f64 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// The fractional MIDI note of a pitch frame's f0 `hz` (A4 = 440 Hz = 69); `None` for a non-positive or non-finite
/// frequency.
pub(crate) fn f0_to_midi(hz: f32) -> Option<f32> {
    (hz.is_finite() && hz > 0.0).then(|| 69.0 + 12.0 * (hz / 440.0).log2())
}

/// The MIDI pitch of frame `i`, `None` when it is unvoiced.
pub(crate) fn frame_midi(track: &PitchTrack, i: usize) -> Option<f32> {
    track.frames.get(i).and_then(|f| f.f0).and_then(f0_to_midi)
}

/// The track's hop in seconds, `None` when it is not a positive finite number.
pub(crate) fn hop_of(track: &PitchTrack) -> Option<f64> {
    let hop = f64::from(track.hop_seconds);
    (hop.is_finite() && hop > 0.0).then_some(hop)
}

/// `ms` milliseconds as a whole number of frames of `hop` seconds, rounded, at least 1.
pub(crate) fn ms_to_frames(ms: u32, hop: f64) -> usize {
    ((f64::from(ms) / 1000.0 / hop).round() as usize).max(1)
}

/// The runs of consecutive voiced frames (a voiced frame has a usable f0).
pub(crate) fn voiced_runs(track: &PitchTrack) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start = None;
    for i in 0..track.frames.len() {
        match (frame_midi(track, i).is_some(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                runs.push(s..i);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        runs.push(s..track.frames.len());
    }
    runs
}

/// The median of `values` (the mean of the middle two for an even count); `None` when empty.
pub(crate) fn median(values: &[f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    Some(median_sorted(&sorted))
}

/// The median of an ascending slice that is not empty (the mean of the middle two for an even
/// count).
pub(crate) fn median_sorted(sorted: &[f32]) -> f32 {
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        0.5 * (sorted[mid - 1] + sorted[mid])
    }
}

/// A centred median filter of `MEDIAN_FRAMES`, its window clipped at the ends.
pub(crate) fn median_smooth(midi: &[f32]) -> Vec<f32> {
    median_filter(midi, MEDIAN_FRAMES / 2)
}

/// A centred median filter over `half` frames either side, its window clipped at the ends.
pub(crate) fn median_filter(midi: &[f32], half: usize) -> Vec<f32> {
    // one sort buffer for every window (a window always holds its own frame, so it is not empty)
    let mut window = Vec::with_capacity(2 * half + 1);
    (0..midi.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(midi.len());
            window.clear();
            window.extend_from_slice(&midi[lo..hi]);
            window.sort_by(f32::total_cmp);
            median_sorted(&window)
        })
        .collect()
}

/// `x` clamped to 0..=1, 0 for NaN.
pub(crate) fn clamp01(x: f32) -> f32 {
    if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{run, track_of};

    #[test]
    fn helpers_cover_edges() {
        assert_eq!(f0_to_midi(440.0), Some(69.0));
        assert_eq!(f0_to_midi(0.0), None);
        assert_eq!(f0_to_midi(f32::NAN), None);
        assert_eq!(f0_to_midi(-5.0), None);
        assert_eq!(midi_to_hz(69.0), 440.0);
        assert_eq!(midi_to_hz(81.0), 880.0);
        assert_eq!(hz_to_midi(220.0), 57.0);
        assert!((hz_to_midi(midi_to_hz(60.37)) - 60.37).abs() < 1e-12);
        assert_eq!(ms_to_frames(30, 0.01), 3);
        assert_eq!(ms_to_frames(40, 0.01), 4);
        assert_eq!(ms_to_frames(1, 0.01), 1);
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), Some(2.5));
        assert_eq!(median_sorted(&[1.0, 2.0, 7.0]), 2.0);
        assert_eq!(median_smooth(&[1.0, 9.0, 1.0, 1.0, 1.0]), vec![1.0; 5]);
        assert_eq!(median_smooth(&[]), Vec::<f32>::new());
        assert_eq!(
            median_filter(&[0.0, 3.0, 0.0, 3.0], 1),
            vec![1.5, 0.0, 3.0, 1.5]
        );
        assert_eq!(median_filter(&[2.0, 4.0], 0), vec![2.0, 4.0]);
        assert_eq!(median_filter(&[], 3), Vec::<f32>::new());
        assert_eq!(
            finite_f64(&[0.5, f32::NAN, f32::INFINITY, -0.25]),
            vec![0.5, 0.0, 0.0, -0.25]
        );
        assert_eq!(clamp01(f32::NAN), 0.0);
        assert_eq!(clamp01(2.0), 1.0);
        let mut midi = run(Some(60.0), 3);
        midi.extend(run(None, 2));
        midi.push(Some(62.0));
        assert_eq!(voiced_runs(&track_of(&midi, 1.0)), vec![0..3, 5..6]);
    }
}
