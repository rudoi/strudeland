//! Shared helpers for the unit tests: synth cases from explicit notes, and hand-built tracks.

use crate::consts::ANALYSIS_RATE;
use crate::eval::synth::{Case, SynthSettings, TruthNote};
use crate::pitch::{PitchFrame, PitchTrack, PyinTracker};
use crate::util::midi_to_hz;

/// The hop of a pitch track at the analysis rate, in seconds.
pub fn hop() -> f32 {
    crate::consts::HOP as f32 / ANALYSIS_RATE as f32
}

/// A synth case playing `(start, end, midi)` notes, in seconds; gaps between notes are silence.
pub fn case_of(notes: &[(f64, f64, i32)], settings: SynthSettings) -> Case {
    let truth = notes
        .iter()
        .map(|&(start, end, midi)| TruthNote {
            start,
            end,
            midi,
            bar: 1,
            step: 0,
            steps: 1,
        })
        .collect();
    let last = notes.iter().fold(0.0f64, |m, n| m.max(n.1));
    let bars = (last / 4.0).ceil().max(1.0) as u32;
    let origin = notes.first().map_or(0.0, |n| n.0);
    Case {
        seed: 11,
        settings: SynthSettings {
            lead_in_seconds: origin,
            ..settings
        },
        bpm: 60.0,
        beats_per_bar: 4.0,
        bars,
        tonic_pc: 0,
        minor: false,
        origin_seconds: origin,
        truth,
    }
}

/// The rendered audio of `notes` at the analysis rate, cut a little after the last note (the
/// generator renders whole bars).
pub fn audio(notes: &[(f64, f64, i32)], settings: SynthSettings) -> Vec<f32> {
    let case = case_of(notes, settings);
    let mut samples = case.render(ANALYSIS_RATE);
    let last = notes.iter().fold(0.0f64, |m, n| m.max(n.1));
    let keep = ((last + 0.3) * f64::from(ANALYSIS_RATE)) as usize;
    samples.truncate(keep);
    samples
}

/// A voice with the default harmonics and formants.
pub fn voice() -> SynthSettings {
    SynthSettings::default()
}

/// A pure sine.
pub fn sine() -> SynthSettings {
    SynthSettings {
        harmonics: 1,
        formants: false,
        ..SynthSettings::default()
    }
}

/// The pYIN track of `samples` at the analysis rate.
pub fn track(samples: &[f32]) -> PitchTrack {
    PyinTracker.track(samples, ANALYSIS_RATE)
}

/// A hand-built track at the analysis hop: `Some(midi)` frames are voiced with `prob`.
pub fn track_of(midi: &[Option<f32>], prob: f32) -> PitchTrack {
    PitchTrack {
        hop_seconds: hop(),
        frames: midi
            .iter()
            .map(|m| PitchFrame {
                f0: m.map(|m| midi_to_hz(f64::from(m)) as f32),
                voiced_prob: if m.is_some() { prob } else { 0.0 },
                rms: if m.is_some() { 0.1 } else { 0.0 },
            })
            .collect(),
    }
}

/// `count` frames of `midi` (or unvoiced for `None`).
pub fn run(midi: Option<f32>, count: usize) -> Vec<Option<f32>> {
    vec![midi; count]
}

/// The frequency of a clean tone from the interpolated rising zero crossings between `from` and
/// `to` seconds.
pub fn zero_crossing_hz(samples: &[f32], rate: u32, from: f64, to: f64) -> f64 {
    let sr = f64::from(rate);
    let (a, b) = ((from * sr) as usize, (to * sr) as usize);
    let mut crossings = Vec::new();
    for i in a..b {
        let (x0, x1) = (f64::from(samples[i]), f64::from(samples[i + 1]));
        if x0 < 0.0 && x1 >= 0.0 {
            crossings.push((i as f64 + (-x0) / (x1 - x0)) / sr);
        }
    }
    let cycles = (crossings.len() - 1) as f64;
    cycles / (crossings[crossings.len() - 1] - crossings[0])
}
