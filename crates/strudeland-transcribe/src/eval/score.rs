//! Note-level scoring under mir_eval's COnP rule: a note is correct when its onset is within
//! 50 ms and its pitch within 50 cents of a reference note, matched one to one.

use super::synth::{Case, TruthNote};
use crate::key::{Key, KeySource, root_name};
use crate::notation::{midi_to_name, render_notation};
use crate::report::{Components, Notation, Note};

/// The COnP onset tolerance in seconds.
pub const ONSET_TOLERANCE: f64 = 0.050;
/// The COnP pitch tolerance in cents.
pub const PITCH_TOLERANCE_CENTS: f64 = 50.0;

/// A note reduced to what COnP compares.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScoredNote {
    /// Onset in seconds.
    pub onset: f64,
    /// Fractional MIDI pitch.
    pub midi: f64,
}

impl From<&TruthNote> for ScoredNote {
    fn from(n: &TruthNote) -> Self {
        Self {
            onset: n.start,
            midi: f64::from(n.midi),
        }
    }
}

impl From<&Note> for ScoredNote {
    /// The note's raw start and its semitone plus the deviation from it.
    fn from(n: &Note) -> Self {
        Self {
            onset: n.start,
            midi: f64::from(n.midi) + f64::from(n.cents) / 100.0,
        }
    }
}

/// Matched counts and the scores derived from them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub tp: usize,
    pub fp: usize,
    pub fn_: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

impl Score {
    /// Builds a score from counts; an empty side gives 0.0, never a division by zero.
    pub fn from_counts(tp: usize, fp: usize, fn_: usize) -> Self {
        let ratio = |num: usize, den: usize| {
            if den == 0 {
                0.0
            } else {
                num as f64 / den as f64
            }
        };
        let precision = ratio(tp, tp + fp);
        let recall = ratio(tp, tp + fn_);
        let f1 = if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            0.0
        };
        Self {
            tp,
            fp,
            fn_,
            precision,
            recall,
            f1,
        }
    }
}

/// COnP of `estimate` against `reference`: candidate pairs within both tolerances are taken in
/// order of smallest onset difference, each note used at most once.
pub fn conp(reference: &[ScoredNote], estimate: &[ScoredNote]) -> Score {
    let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
    for (i, r) in reference.iter().enumerate() {
        for (j, e) in estimate.iter().enumerate() {
            let dt = (e.onset - r.onset).abs();
            let cents = (e.midi - r.midi).abs() * 100.0;
            if dt <= ONSET_TOLERANCE && cents <= PITCH_TOLERANCE_CENTS {
                pairs.push((dt, i, j));
            }
        }
    }
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut ref_used = vec![false; reference.len()];
    let mut est_used = vec![false; estimate.len()];
    let mut tp = 0;
    for (_, i, j) in pairs {
        if !ref_used[i] && !est_used[j] {
            ref_used[i] = true;
            est_used[j] = true;
            tp += 1;
        }
    }
    Score::from_counts(tp, estimate.len() - tp, reference.len() - tp)
}

/// Micro-average: the counts are summed over the cases and the scores recomputed.
pub fn micro_average(scores: &[Score]) -> Score {
    let (tp, fp, fn_) = scores
        .iter()
        .fold((0, 0, 0), |(a, b, c), s| (a + s.tp, b + s.fp, c + s.fn_));
    Score::from_counts(tp, fp, fn_)
}

/// The case's own key (its generated tonic and mode), as a detected key without scores.
pub fn case_key(case: &Case) -> Key {
    let scale = if case.minor { "minor" } else { "major" };
    Key {
        name: format!("{} {scale}", root_name(case.tonic_pc)),
        tonic_pc: case.tonic_pc % 12,
        scale: scale.to_string(),
        confidence: None,
        runner_up: None,
        correlation: None,
        source: KeySource::Detected,
    }
}

/// The ground truth as quantized notes on the sixteenth grid (bar, step and length from the
/// generator, before jitter).
pub fn truth_notes(truth: &[TruthNote]) -> Vec<Note> {
    truth
        .iter()
        .map(|t| Note {
            start: t.start,
            end: t.end,
            bar: Some(t.bar),
            step: Some(t.step),
            steps: Some(t.steps),
            midi: t.midi,
            name: midi_to_name(t.midi),
            degree: None,
            cents: 0.0,
            confidence: 1.0,
            components: Components {
                voiced: 1.0,
                stability: 1.0,
                fit: 1.0,
                onset: None,
            },
            snap_error_ms: Some(0.0),
            tied_to_next: false,
            alternative: None,
            early: false,
        })
        .collect()
}

/// The generator's own notation of `case` on the sixteenth grid, with `key` for the degree form:
/// from bar 0 when the case has a pickup, else from bar 1.
pub fn truth_notation(case: &Case, key: Option<&Key>) -> Notation {
    let first_bar = if case.truth.iter().any(|n| n.bar == 0) {
        0
    } else {
        1
    };
    render_notation(
        &truth_notes(&case.truth),
        case.steps_per_bar(),
        first_bar,
        case.bars,
        key,
    )
}

/// True when `estimate` equals the generator's notation of `case` rendered with the same `key`
/// (the transcription's own, so a wrong key does not count against the notes): the note form,
/// and the degree form and scale when there is a key.
pub fn exact_notation(estimate: Option<&Notation>, case: &Case, key: Option<&Key>) -> bool {
    estimate.is_some_and(|e| *e == truth_notation(case, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::synth::STEPS_PER_BAR;

    fn n(onset: f64, midi: f64) -> ScoredNote {
        ScoredNote { onset, midi }
    }

    #[test]
    fn conp_perfect_is_1() {
        let notes = [n(0.0, 60.0), n(0.5, 62.0), n(1.0, 64.0)];
        let s = conp(&notes, &notes);
        assert_eq!((s.tp, s.fp, s.fn_), (3, 0, 0));
        assert_eq!((s.precision, s.recall, s.f1), (1.0, 1.0, 1.0));
    }

    #[test]
    fn conp_wrong_pitch_beyond_50_cents_misses() {
        let reference = [n(0.0, 60.0), n(0.5, 62.0)];
        // 49 cents is a hit, 51 cents a miss
        let s = conp(&reference, &[n(0.0, 60.49), n(0.5, 62.51)]);
        assert_eq!((s.tp, s.fp, s.fn_), (1, 1, 1), "{s:?}");
        assert_eq!(s.f1, 0.5);
    }

    #[test]
    fn conp_onset_beyond_50_ms_misses() {
        let reference = [n(1.0, 60.0), n(2.0, 60.0)];
        let s = conp(&reference, &[n(1.049, 60.0), n(2.051, 60.0)]);
        assert_eq!((s.tp, s.fp, s.fn_), (1, 1, 1), "{s:?}");
    }

    #[test]
    fn conp_one_to_one_matching() {
        // one estimate cannot match two references, nor two estimates one reference
        let s = conp(&[n(1.0, 60.0), n(1.02, 60.0)], &[n(1.01, 60.0)]);
        assert_eq!(
            (s.tp, s.fp, s.fn_),
            (1, 0, 1),
            "one estimate, two references"
        );
        let s = conp(&[n(1.0, 60.0)], &[n(1.01, 60.0), n(1.03, 60.0)]);
        assert_eq!(
            (s.tp, s.fp, s.fn_),
            (1, 1, 0),
            "two estimates, one reference"
        );
        // the smallest onset difference wins: the reference at 1.04 takes the estimate at 1.04,
        // leaving the one at 1.0 for the reference at 0.98
        let s = conp(
            &[n(0.98, 60.0), n(1.04, 60.0)],
            &[n(1.0, 60.0), n(1.04, 60.0)],
        );
        assert_eq!(s.tp, 2, "{s:?}");
    }

    #[test]
    fn empty_reference_and_estimate() {
        let s = conp(&[], &[]);
        assert_eq!((s.tp, s.fp, s.fn_), (0, 0, 0));
        assert_eq!((s.precision, s.recall, s.f1), (0.0, 0.0, 0.0));
        let s = conp(&[], &[n(0.0, 60.0)]);
        assert_eq!((s.precision, s.recall, s.f1), (0.0, 0.0, 0.0));
        assert_eq!(s.fp, 1);
        let s = conp(&[n(0.0, 60.0)], &[]);
        assert_eq!((s.precision, s.recall, s.f1), (0.0, 0.0, 0.0));
        assert_eq!(s.fn_, 1);
        assert_eq!(micro_average(&[]).f1, 0.0);
    }

    #[test]
    fn micro_average_sums_counts_not_scores() {
        let a = Score::from_counts(1, 0, 0);
        let b = Score::from_counts(1, 9, 9);
        let m = micro_average(&[a, b]);
        assert_eq!((m.tp, m.fp, m.fn_), (2, 9, 9));
        assert_eq!(m.precision, 2.0 / 11.0);
        assert_eq!(m.recall, 2.0 / 11.0);
        let mean_of_f1 = (a.f1 + b.f1) / 2.0;
        assert!(
            (m.f1 - mean_of_f1).abs() > 0.1,
            "not the mean of the scores"
        );
    }

    #[test]
    fn exact_notation_check() {
        let case = Case::generate(1000, &crate::eval::synth::SynthSettings::default());
        let key = case_key(&case);
        let truth = truth_notation(&case, Some(&key));
        // the generator's notes rendered through the notation are an exact match
        assert!(exact_notation(Some(&truth), &case, Some(&key)));
        // the bars: one bracket group per bar
        assert!(truth.note.starts_with("<[") && truth.note.ends_with("]>"));
        assert_eq!(truth.note.matches('[').count(), case.bars as usize);
        assert!(truth.n.is_some() && truth.scale.is_some());
        // one note a semitone out is not
        let mut notes = truth_notes(&case.truth);
        notes[3].midi += 1;
        let wrong = render_notation(&notes, STEPS_PER_BAR, 1, case.bars, Some(&key));
        assert!(!exact_notation(Some(&wrong), &case, Some(&key)));
        // nor is a note a step late
        let mut notes = truth_notes(&case.truth);
        let last = notes.len() - 1;
        if notes[last].step.unwrap() + notes[last].steps.unwrap() < STEPS_PER_BAR {
            *notes[last].step.as_mut().unwrap() += 1;
        } else {
            *notes[last].steps.as_mut().unwrap() -= 1;
        }
        let late = render_notation(&notes, STEPS_PER_BAR, 1, case.bars, Some(&key));
        assert!(!exact_notation(Some(&late), &case, Some(&key)));
        // no notation at all
        assert!(!exact_notation(None, &case, Some(&key)));
        // without a key only the note form is compared, and the degree form is absent
        let plain = truth_notation(&case, None);
        assert_eq!(plain.note, truth.note);
        assert!(exact_notation(Some(&plain), &case, None));
        assert!(
            !exact_notation(Some(&truth), &case, None),
            "a degree form where none is due"
        );
        // the key names the case's tonic and mode
        assert_eq!(key.tonic_pc, case.tonic_pc);
        assert!(
            key.name
                .ends_with(if case.minor { " minor" } else { " major" })
        );
    }

    #[test]
    fn converts_truth_and_report_notes() {
        let t = TruthNote {
            start: 1.5,
            end: 2.0,
            midi: 64,
            bar: 1,
            step: 0,
            steps: 4,
        };
        assert_eq!(ScoredNote::from(&t), n(1.5, 64.0));
        let note = Note {
            start: 1.51,
            end: 2.0,
            bar: Some(1),
            step: Some(0),
            steps: Some(4),
            midi: 64,
            name: "e4".into(),
            degree: None,
            cents: -20.0,
            confidence: 1.0,
            components: crate::report::Components {
                voiced: 1.0,
                stability: 1.0,
                fit: 1.0,
                onset: None,
            },
            snap_error_ms: None,
            tied_to_next: false,
            alternative: None,
            early: false,
        };
        let s = ScoredNote::from(&note);
        assert_eq!(s.onset, 1.51);
        assert!((s.midi - 63.8).abs() < 1e-6);
        assert_eq!(conp(&[(&t).into()], &[s]).tp, 1);
    }
}
