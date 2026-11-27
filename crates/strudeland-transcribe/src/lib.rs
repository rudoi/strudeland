//! Melody transcription for a monophonic voice: a sung or hummed take in, notes quantized to a
//! bar grid and Strudel mini-notation out.
//!
//! The crate is pure: it takes samples and a grid hint, never paths, sidecars, devices or the
//! network, and it holds no global state. The stages (`load`, pitch tracking, segmentation, grid,
//! quantizing, key, notation) are plain functions over plain data, and the result is the
//! serializable [`Transcription`]. `eval` (behind the `eval` feature, and in the crate's own unit
//! tests) is test support: a seeded synthetic melody generator and a note-level scorer.

pub mod consts;
mod error;
#[cfg(any(test, feature = "eval"))]
pub mod eval;
mod grid;
mod key;
pub mod load;
mod notation;
mod onset;
mod pitch;
mod quantize;
pub mod report;
mod segment;
mod suggest;
#[cfg(test)]
mod test_support;
mod tuning;
mod util;

pub use error::TranscribeError;
pub use grid::{GridHint, Hinted, Resolved, parse_meter};
pub use key::{Key, KeySource, NoKey, parse_key_override};
pub use quantize::GridDivision;
pub use report::Transcription;

use std::borrow::Cow;

use consts::{ANALYSIS_RATE, FRAME, MAX_NOTATION_BARS, TUNING_WARN_CENTS};
use grid::{Grid, GridOutcome, meter_label, resolve_grid};
use key::detect_key;
use notation::{Window, WindowStart, midi_to_name, render_notation};
use onset::onset_strength_of_signal;
use pitch::{PitchTracker, PyinTracker};
use quantize::quantize;
use report::{DebugFrame, GridReport, GridSource, Note, TuningReport, TuningSource};
use segment::{RawNote, segment};
use tuning::estimate_tuning;
use util::finite_f64;

/// What `transcribe` is asked to do beyond the grid hint.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Options {
    /// `--grid`; default sixteenths.
    pub grid: GridDivision,
    /// `--key` ([`parse_key_override`]): skips detection.
    pub key: Option<Key>,
    /// `--tuning` in cents, positive sharp; `Some(0.0)` turns tuning off.
    pub tuning_cents: Option<f32>,
    /// `--shift` in milliseconds, positive moves the origin later.
    pub shift_ms: f64,
    /// Memo seconds of `samples[0]` (0.0 for the whole memo).
    pub span_start_seconds: f64,
    /// The memo's whole length in seconds, for `Bar1PastEnd`; `None` is the end of `samples`
    /// (`span_start_seconds` plus their duration).
    pub memo_seconds: Option<f64>,
    /// `--bars A-B` as `(A, B)`: memo bars, 1-based and inclusive, `1 <= A <= B`. The
    /// notation then covers exactly bars A to B, rest bars included, whatever bar the samples
    /// start in. `None` notates from the bar the samples start in (or the pickup bar) to the
    /// last bar with a note.
    pub bars: Option<(u32, u32)>,
    /// Fill `Transcription::{frames, onsets}`.
    pub debug: bool,
}

/// The warning when the input is shorter than one analysis frame.
pub const TOO_SHORT_WARNING: &str = "the take is too short to analyse";
/// The warning when no notes are found.
pub const NO_NOTES_WARNING: &str = "no notes found";

/// Transcribes mono `samples` at `sample_rate` Hz (the span already cut; `opts.span_start_seconds`
/// places it in the memo): resample to 16 kHz, pYIN, tuning, onsets, segmentation, grid,
/// quantizing, key and notation. Non-finite samples count as silence, and hint values that are
/// not finite (or a tempo or meter that is not positive) as absent. A sample rate outside
/// `load::MIN_SOURCE_RATE..=load::MAX_SOURCE_RATE` or a non-finite option is `BadOption`; input
/// shorter than one analysis frame after resampling is an empty `Ok` with a warning.
pub fn transcribe(
    samples: &[f32],
    sample_rate: u32,
    hint: &GridHint,
    opts: &Options,
) -> Result<Transcription, TranscribeError> {
    let bad = |what: &str, value: &dyn std::fmt::Display| {
        TranscribeError::BadOption(format!("{what} must be a finite number (got {value})"))
    };
    let span_start = opts.span_start_seconds;
    if !span_start.is_finite() {
        return Err(bad("the span start", &span_start));
    }
    if !opts.shift_ms.is_finite() {
        return Err(bad("--shift", &opts.shift_ms));
    }
    if let Some(t) = opts.tuning_cents.filter(|t| !t.is_finite()) {
        return Err(bad("--tuning", &t));
    }
    if let Some(m) = opts.memo_seconds.filter(|m| !m.is_finite()) {
        return Err(bad("the memo length", &m));
    }
    if let Some((from, to)) = opts.bars
        && (from == 0 || from > to)
    {
        return Err(TranscribeError::BadOption(format!(
            "the bars must be 1 <= A <= B (got {from}-{to})"
        )));
    }
    let hint = hint.validated();
    let clean: Cow<[f32]> = if samples.iter().all(|s| s.is_finite()) {
        Cow::Borrowed(samples)
    } else {
        Cow::Owned(
            samples
                .iter()
                .map(|s| if s.is_finite() { *s } else { 0.0 })
                .collect(),
        )
    };
    let analysis = load::resample_to_analysis(&clean, sample_rate)?;
    let duration = samples.len() as f64 / f64::from(sample_rate);
    let memo_seconds = opts.memo_seconds.unwrap_or(span_start + duration);
    let tuning_given = opts.tuning_cents;
    let tuning_source = match tuning_given {
        Some(_) => TuningSource::Flag,
        None => TuningSource::Estimated,
    };

    let mut out = Transcription {
        memo: None,
        duration,
        grid: None,
        tuning: TuningReport {
            cents: tuning_given.unwrap_or(0.0),
            source: tuning_source,
        },
        key: None,
        notation: None,
        notes: Vec::new(),
        dropped: Vec::new(),
        timing_spread: None,
        warnings: Vec::new(),
        no_key: None,
        backing: None,
        frames: None,
        onsets: None,
    };
    if analysis.len() < FRAME {
        out.warnings.push(TOO_SHORT_WARNING.to_string());
        if opts.debug {
            out.frames = Some(Vec::new());
            out.onsets = Some(Vec::new());
        }
        return Ok(out);
    }

    // one f64 copy of the analysis signal for the pitch tracker and the onset stage
    let signal = finite_f64(&analysis);
    drop(analysis);
    let track = PyinTracker.track_signal(&signal, ANALYSIS_RATE);
    let tuning = match tuning_given {
        Some(c) => c,
        None => {
            let c = estimate_tuning(&track);
            if c.abs() > TUNING_WARN_CENTS {
                out.warnings.push(format!(
                    "tuning is {c:.0} cents off the nearest semitone; the note names may be a semitone out. Pass --tuning"
                ));
            }
            c
        }
    };
    out.tuning.cents = tuning;
    let onsets = onset_strength_of_signal(&signal);
    let mut raw = segment(&track, &onsets, tuning);
    for n in &mut raw {
        n.start += span_start;
        n.end += span_start;
    }

    // (steps per bar, what fixes the notation's first bar)
    let mut notation_grid = None;
    match resolve_grid(&hint, &onsets, span_start, memo_seconds, opts.shift_ms)? {
        GridOutcome::Known(grid, warnings) => {
            out.warnings.extend(warnings);
            let q = quantize(&raw, &grid, opts.grid)?;
            let division = suggest::suggest_division(&raw, &grid, opts.grid);
            // the shift on the suggested division when there is one, so the two go together
            let (shift_division, spread) = match &division {
                Some(s) => (s.division, s.spread_ms),
                None => (opts.grid, q.timing_spread_ms),
            };
            let shift = suggest::suggest_shift(&raw, &grid, shift_division, spread);
            out.warnings.extend(q.warnings);
            if let Some(s) = &division {
                out.warnings
                    .push(suggest::division_warning(s, q.timing_spread_ms));
            }
            if let Some(delta) = shift {
                out.warnings.push(suggest::shift_warning(
                    delta,
                    opts.shift_ms,
                    division.as_ref().map(|s| s.division),
                ));
            }
            out.timing_spread = (!q.notes.is_empty()).then_some(q.timing_spread_ms);
            out.notes = q.notes;
            out.dropped = q.dropped;
            let start = match opts.bars {
                Some((from, to)) => WindowStart::Requested { from, to },
                None => WindowStart::SpanBar(span_bar(span_start, &grid)),
            };
            notation_grid = Some((q.steps_per_bar, start));
            out.grid = Some(GridReport {
                bpm: grid.bpm,
                meter: hint
                    .meter
                    .clone()
                    .unwrap_or_else(|| meter_label(grid.beats_per_bar)),
                beats_per_bar: grid.beats_per_bar,
                origin: grid.origin_seconds,
                source: GridSource {
                    tempo: grid.tempo_source,
                    phase: grid.phase_source,
                },
                confidence: grid.confidence,
                steps_per_bar: q.steps_per_bar,
                division: opts.grid,
                suggested_division: division.map(|s| s.division),
                suggested_shift_ms: shift.map(|delta| opts.shift_ms + delta),
            });
        }
        GridOutcome::Unresolved(warnings) => {
            out.warnings.extend(warnings);
            out.notes = raw.iter().map(unquantized).collect();
        }
    }
    if out.notes.is_empty() {
        out.warnings.push(NO_NOTES_WARNING.to_string());
    }

    out.key = match &opts.key {
        Some(k) => Some(k.clone()),
        None => match detect_key(&out.notes) {
            Ok(k) => Some(k),
            Err(reason) => {
                out.warnings.push(format!("no key: {reason}"));
                out.no_key = Some(reason);
                None
            }
        },
    };
    notation::fill_degrees(&mut out.notes, out.key.as_ref());
    if let Some((spb, start)) = notation_grid {
        // the notes the notation can show: not those more than a bar before bar 1
        let notated: Vec<Note> = out
            .notes
            .iter()
            .filter(|n| !n.early && n.step.is_some() && n.steps.is_some())
            .cloned()
            .collect();
        match notation::window(start, &notated) {
            Window::TooLong { last_bar } => out.warnings.push(format!(
                "the notes reach bar {last_bar}; notation is limited to {MAX_NOTATION_BARS} bars"
            )),
            Window::Bars { first, last } => {
                out.notation = Some(render_notation(
                    &notated,
                    spb,
                    first,
                    last,
                    out.key.as_ref(),
                ));
            }
            Window::Empty => {}
        }
    }

    if opts.debug {
        let hop = f64::from(track.hop_seconds);
        out.frames = Some(
            track
                .frames
                .iter()
                .enumerate()
                .map(|(i, f)| DebugFrame {
                    t: i as f64 * hop,
                    f0: f.f0,
                    voiced: f.voiced_prob,
                    rms: f.rms,
                })
                .collect(),
        );
        out.onsets = Some(onsets.peaks.clone());
    }
    Ok(out)
}

/// Slack when a whole or seconds span starts on a barline, in seconds, so a start computed as the
/// barline's time, but a rounding short of it, starts in that bar.
const BARLINE_SLACK_SECONDS: f64 = 1e-6;

/// The bar of `grid` that memo time `span_start` falls in; 0 before bar 1 or when it is not a
/// number (the notation then starts at the pickup bar, bar 0, when it has notes, else at bar 1).
fn span_bar(span_start: f64, grid: &Grid) -> u32 {
    let bar = grid.beats_per_bar * 60.0 / grid.bpm;
    let index = ((span_start - grid.origin_seconds + BARLINE_SLACK_SECONDS) / bar).floor() + 1.0;
    // the cast saturates: below 1 is 0, past u32::MAX is u32::MAX
    if index.is_finite() { index as u32 } else { 0 }
}

/// A raw note (memo seconds) without a grid: no bar, step, length or snap error.
fn unquantized(n: &RawNote) -> Note {
    let rounded = n.midi.round();
    let midi = rounded as i32;
    Note {
        start: n.start,
        end: n.end,
        bar: None,
        step: None,
        steps: None,
        midi,
        name: midi_to_name(midi),
        degree: None,
        cents: (n.midi - rounded) * 100.0,
        confidence: n.confidence,
        components: n.components.clone(),
        snap_error_ms: None,
        tied_to_next: false,
        alternative: report::Alternative::of(midi, (n.midi - rounded) * 100.0),
        early: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::score::{case_key, conp, exact_notation, truth_notation, truth_notes};
    use crate::eval::synth::{Case, SplitMix64, SynthSettings};
    use crate::grid::UNRESOLVED_WARNING;
    use crate::onset::onset_strength;
    use crate::test_support::{audio, case_of, voice};

    /// The grid a case was generated on, as a sidecar would give it.
    fn sidecar(case: &Case) -> GridHint {
        GridHint {
            bpm: Some(Hinted::sidecar(case.bpm)),
            beats_per_bar: Some(Hinted::sidecar(case.beats_per_bar)),
            origin_seconds: Some(Hinted::sidecar(case.origin_seconds)),
            ..GridHint::default()
        }
    }

    /// 120 bpm 4/4 from `origin`: a sixteenth is 0.125 s.
    fn grid_at(origin: f64) -> GridHint {
        GridHint {
            bpm: Some(Hinted::sidecar(120.0)),
            beats_per_bar: Some(Hinted::sidecar(4.0)),
            origin_seconds: Some(Hinted::sidecar(origin)),
            ..GridHint::default()
        }
    }

    /// Seven notes from 0.5 s at 120 bpm: C major, the tonic held.
    fn melody() -> Vec<(f64, f64, i32)> {
        vec![
            (0.5, 1.0, 60),
            (1.0, 1.25, 62),
            (1.25, 1.5, 64),
            (1.5, 1.75, 65),
            (1.75, 2.0, 67),
            (2.0, 2.25, 64),
            (2.25, 2.5, 62),
            (2.5, 3.5, 60),
        ]
    }

    fn run(samples: &[f32], rate: u32, hint: &GridHint, opts: &Options) -> Transcription {
        transcribe(samples, rate, hint, opts).expect("transcribes")
    }

    #[test]
    fn whole_pipeline_on_a_clean_melody_matches_notation() {
        let case = Case::generate(1000, &SynthSettings::default());
        let samples = case.render(48_000);
        let t = run(&samples, 48_000, &sidecar(&case), &Options::default());
        let key = t.key.clone();
        let truth = truth_notation(&case, key.as_ref());
        assert!(
            exact_notation(t.notation.as_ref(), &case, key.as_ref()),
            "seed 1000\n got  {:?}\n want {:?}",
            t.notation,
            truth
        );
        let reference: Vec<_> = case.truth.iter().map(Into::into).collect();
        let estimate: Vec<_> = t.notes.iter().map(Into::into).collect();
        assert_eq!(conp(&reference, &estimate).f1, 1.0, "seed 1000");
        let grid = t.grid.as_ref().expect("grid");
        assert_eq!((grid.bpm, grid.beats_per_bar), (case.bpm, 4.0));
        assert_eq!(grid.meter, "4/4");
        assert_eq!(grid.steps_per_bar, 16);
        assert_eq!(grid.division, GridDivision::Sixteenth);
        assert_eq!(grid.origin, case.origin_seconds);
        assert_eq!(
            (grid.source.tempo, grid.source.phase),
            (Resolved::Sidecar, Resolved::Sidecar)
        );
        assert_eq!(grid.confidence, None);
        assert_eq!(t.tuning.source, TuningSource::Estimated);
        assert!(t.tuning.cents.abs() < 5.0, "tuning {}", t.tuning.cents);
        // E minor against G major: the detector may decline; either way the report is coherent
        match &t.key {
            Some(key) => {
                assert_eq!(key.source, KeySource::Detected);
                assert!(t.notes.iter().all(|n| n.degree.is_some()));
            }
            None => {
                let reason = t.no_key.expect("a reason");
                assert!(
                    t.warnings.contains(&format!("no key: {reason}")),
                    "{:?}",
                    t.warnings
                );
                assert!(t.notes.iter().all(|n| n.degree.is_none()));
                assert_eq!(t.notation.as_ref().unwrap().n, None);
            }
        }
        // the case's own key as --key: the degree form matches the generator's too
        let key = case_key(&case);
        let opts = Options {
            key: Some(parse_key_override(&format!("{}:{}", key.root(), key.scale)).unwrap()),
            ..Options::default()
        };
        let keyed = run(&samples, 48_000, &sidecar(&case), &opts);
        let flag_key = keyed.key.clone().unwrap();
        assert_eq!(flag_key.tonic_pc, key.tonic_pc);
        assert!(
            exact_notation(keyed.notation.as_ref(), &case, Some(&flag_key)),
            "seed 1000 with its key\n got  {:?}\n want {:?}",
            keyed.notation,
            truth_notation(&case, Some(&flag_key))
        );
        assert!(t.timing_spread.is_some());
        assert_eq!(t.duration, samples.len() as f64 / 48_000.0);
        assert!(t.frames.is_none() && t.onsets.is_none());
        assert!(t.memo.is_none() && t.backing.is_none());
    }

    #[test]
    fn never_panics_on_odd_input() {
        let hints = [GridHint::default(), grid_at(0.0), grid_at(1e9)];
        let mut noise = SplitMix64::new(7);
        let tone = |hz: f64, rate: u32, seconds: f64| -> Vec<f32> {
            (0..(seconds * f64::from(rate)) as usize)
                .map(|i| {
                    (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin()) as f32
                })
                .collect()
        };
        let inputs: Vec<(&str, Vec<f32>, u32)> = vec![
            ("empty", vec![], 16_000),
            ("one sample", vec![0.5], 16_000),
            ("silence", vec![0.0; 32_000], 16_000),
            (
                "full-scale noise",
                (0..32_000)
                    .map(|_| noise.uniform(-1.0, 1.0) as f32)
                    .collect(),
                16_000,
            ),
            ("1 Hz", tone(1.0, 16_000, 2.0), 16_000),
            ("7 kHz sine", tone(7_000.0, 16_000, 2.0), 16_000),
            (
                "8 kHz rate",
                audio(&melody(), voice())
                    .iter()
                    .step_by(2)
                    .copied()
                    .collect(),
                8_000,
            ),
            ("192 kHz rate", tone(440.0, 192_000, 1.5), 192_000),
            (
                "non-finite samples",
                vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0]
                    .into_iter()
                    .cycle()
                    .take(20_000)
                    .collect(),
                16_000,
            ),
            ("full scale DC", vec![1.0; 20_000], 16_000),
        ];
        for (name, samples, rate) in &inputs {
            for hint in &hints {
                for grid in [GridDivision::Sixteenth, GridDivision::SixteenthTriplet] {
                    let opts = Options {
                        grid,
                        ..Options::default()
                    };
                    match transcribe(samples, *rate, hint, &opts) {
                        Ok(t) => {
                            assert!(t.duration.is_finite(), "{name}");
                            serde_json::to_string(&t).expect(name);
                        }
                        // a hint origin past the end is the only error these inputs give
                        Err(e) => assert!(
                            matches!(e, TranscribeError::Bar1PastEnd { .. }),
                            "{name}: {e}"
                        ),
                    }
                }
            }
        }
        // rates out of range are BadOption, never a panic or a division by zero
        for rate in [0u32, 1, 999, u32::MAX] {
            let err =
                transcribe(&[0.0; 4096], rate, &grid_at(0.0), &Options::default()).unwrap_err();
            assert!(
                matches!(err, TranscribeError::BadOption(_)),
                "rate {rate}: {err}"
            );
        }
        // non-finite options are BadOption
        let bad = [
            Options {
                span_start_seconds: f64::NAN,
                ..Options::default()
            },
            Options {
                shift_ms: f64::INFINITY,
                ..Options::default()
            },
            Options {
                tuning_cents: Some(f32::NAN),
                ..Options::default()
            },
            Options {
                memo_seconds: Some(f64::NEG_INFINITY),
                ..Options::default()
            },
        ];
        for opts in &bad {
            let err = transcribe(&[0.0; 4096], 16_000, &grid_at(0.0), opts).unwrap_err();
            assert!(
                matches!(err, TranscribeError::BadOption(_)),
                "{opts:?}: {err}"
            );
        }
        // extreme but finite options
        let samples = audio(&melody(), voice());
        for opts in [
            Options {
                tuning_cents: Some(1e30),
                ..Options::default()
            },
            Options {
                shift_ms: -1e12,
                ..Options::default()
            },
            Options {
                span_start_seconds: 1e12,
                ..Options::default()
            },
        ] {
            let t = transcribe(&samples, 16_000, &grid_at(0.5), &opts).expect("finite options");
            serde_json::to_string(&t).unwrap();
            if opts.span_start_seconds > 0.0 {
                assert_eq!(t.notation, None, "bars far past the limit");
                assert!(
                    t.warnings
                        .iter()
                        .any(|w| w.ends_with("notation is limited to 100000 bars")),
                    "{:?}",
                    t.warnings
                );
            }
        }
    }

    #[test]
    fn deterministic_output() {
        let case = Case::generate(
            1005,
            &SynthSettings {
                vibrato_cents: 50.0,
                breath_noise_db: Some(-30.0),
                ..SynthSettings::default()
            },
        );
        let samples = case.render(48_000);
        let opts = Options {
            debug: true,
            ..Options::default()
        };
        let a = serde_json::to_string(&run(&samples, 48_000, &GridHint::default(), &opts)).unwrap();
        let b = serde_json::to_string(&run(&samples, 48_000, &GridHint::default(), &opts)).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn a_span_later_in_the_memo_starts_its_notation_at_its_bar() {
        let case = Case::generate(1000, &SynthSettings::default());
        let samples = case.render(48_000);
        let bar = case.bar_seconds();
        // bars 5 to 8, as the CLI cuts them for --bars 5-8
        let (from, to) = (
            case.origin_seconds + 4.0 * bar,
            case.origin_seconds + 8.0 * bar,
        );
        let span = &samples[(from * 48_000.0).round() as usize..(to * 48_000.0).round() as usize];
        let opts = Options {
            span_start_seconds: from,
            memo_seconds: Some(samples.len() as f64 / 48_000.0),
            ..Options::default()
        };
        let t = run(span, 48_000, &sidecar(&case), &opts);
        let notation = t.notation.as_ref().expect("notation");
        assert_eq!((notation.first_bar, notation.last_bar), (5, 8));
        assert!(t.notes.iter().all(|n| (5..=8).contains(&n.bar.unwrap())));
        let truth = render_notation(&truth_notes(&case.truth), 16, 5, 8, None);
        assert_eq!(notation.note, truth.note, "bars 5-8 without leading rests");
        assert!(!notation.note.starts_with("<~"), "{}", notation.note);
        // the whole memo starts at bar 1
        let whole = run(&samples, 48_000, &sidecar(&case), &Options::default());
        assert_eq!(whole.notation.unwrap().first_bar, 1);
    }

    #[test]
    fn a_requested_last_bar_keeps_trailing_rest_bars() {
        // notes in bars 1 to 6 of an eight-bar take; bars 5 to 8 as --bars 5-8 cuts them
        let mut case = Case::generate(1000, &SynthSettings::default());
        case.truth.retain(|n| n.bar <= 6);
        case.bars = 8;
        let samples = case.render(48_000);
        let bar = case.bar_seconds();
        let (from, to) = (
            case.origin_seconds + 4.0 * bar,
            case.origin_seconds + 8.0 * bar,
        );
        let span = &samples[(from * 48_000.0).round() as usize..(to * 48_000.0).round() as usize];
        let opts = Options {
            span_start_seconds: from,
            memo_seconds: Some(samples.len() as f64 / 48_000.0),
            bars: Some((5, 8)),
            ..Options::default()
        };
        let t = run(span, 48_000, &sidecar(&case), &opts);
        let notation = t.notation.as_ref().expect("notation");
        assert_eq!((notation.first_bar, notation.last_bar), (5, 8));
        let truth = render_notation(&truth_notes(&case.truth), 16, 5, 8, None);
        assert_eq!(notation.note, truth.note);
        assert!(notation.note.ends_with("] ~ ~>"), "{}", notation.note);
        // without it the notation ends at the last bar with a note
        let opts = Options { bars: None, ..opts };
        let t = run(span, 48_000, &sidecar(&case), &opts);
        assert_eq!(t.notation.unwrap().last_bar, 6);
        // a requested bar past the limit warns instead
        let opts = Options {
            bars: Some((5, MAX_NOTATION_BARS + 1)),
            ..opts
        };
        let t = run(span, 48_000, &sidecar(&case), &opts);
        assert_eq!(t.notation, None);
    }

    #[test]
    fn a_bars_request_starts_at_its_bar_when_the_cut_rounds_early() {
        // 110 bpm at 48 kHz: cut at whole frames, as the loader cuts it, the span for --bars 2-3
        // starts more than the barline slack before bar 2
        let beat = 60.0 / 110.0;
        let midis = [[60, 62, 64, 65], [67, 69, 71, 72], [72, 71, 69, 67]];
        let notes: Vec<(f64, f64, i32)> = midis
            .iter()
            .enumerate()
            .flat_map(|(b, bar)| {
                bar.iter().enumerate().map(move |(i, &midi)| {
                    let start = 0.5 + (b as f64 * 4.0 + i as f64) * beat;
                    (start, start + 0.9 * beat, midi)
                })
            })
            .collect();
        let mut case = case_of(&notes, voice());
        case.bpm = 110.0;
        case.beats_per_bar = 4.0;
        case.origin_seconds = 0.5;
        case.bars = 3;
        let samples = case.render(48_000);
        let hint = sidecar(&case);
        let (from, to) = hint.bar_span(2, 3, 0.0).unwrap();
        let len = samples.len() as f64;
        let a = (from * 48_000.0).round();
        let b = (to * 48_000.0).round().min(len);
        let start = a / 48_000.0;
        assert!(
            from - start > BARLINE_SLACK_SECONDS,
            "{from} cut at {start}"
        );
        let span = &samples[a as usize..b as usize];
        let seconds = Options {
            span_start_seconds: start,
            memo_seconds: Some(len / 48_000.0),
            ..Options::default()
        };
        // a seconds span from the rounded start begins in bar 1: a leading rest bar
        let by_seconds = run(span, 48_000, &hint, &seconds);
        let s = by_seconds.notation.as_ref().expect("notation");
        assert_eq!((s.first_bar, s.last_bar), (1, 3));
        assert!(s.note.starts_with("<~ "), "{}", s.note);
        // the bars request is exactly bars 2 to 3, the same notes without the rest bar
        let opts = Options {
            bars: Some((2, 3)),
            ..seconds.clone()
        };
        let bars = run(span, 48_000, &hint, &opts);
        assert_eq!(bars.notes, by_seconds.notes);
        let n = bars.notation.as_ref().expect("notation");
        assert_eq!((n.first_bar, n.last_bar), (2, 3), "{n:?}");
        assert_eq!(s.note, format!("<~ {}", &n.note[1..]));
    }

    #[test]
    fn span_bar_is_the_bar_the_span_starts_in() {
        let grid = Grid {
            bpm: 120.0,
            beats_per_bar: 4.0,
            origin_seconds: 1.0,
            tempo_source: Resolved::Sidecar,
            phase_source: Resolved::Sidecar,
            confidence: None,
        };
        // a bar is 2 s from 1.0 s
        for (start, bar) in [
            (0.0, 0),
            (0.5, 0),
            (1.0, 1),
            (2.999, 1),
            (3.0, 2),
            (3.0 - 1e-9, 2),
            (9.0, 5),
            (10.5, 5),
            (-5.0, 0),
            (f64::NAN, 0),
            (f64::INFINITY, 0),
            (1e300, u32::MAX),
        ] {
            assert_eq!(span_bar(start, &grid), bar, "start {start}");
        }
    }

    #[test]
    fn unresolved_grid_reports_unquantized_notes() {
        // three notes with gaps: three onsets, too few to infer a grid
        let notes = [(0.5, 1.0, 60), (1.5, 2.0, 64), (2.5, 3.0, 67)];
        let samples = audio(&notes, voice());
        let t = run(&samples, 16_000, &GridHint::default(), &Options::default());
        assert_eq!(t.grid, None);
        assert_eq!(t.notation, None);
        assert_eq!(t.timing_spread, None);
        assert_eq!(
            t.warnings,
            vec![
                UNRESOLVED_WARNING.to_string(),
                "no key: fewer than 6 notes".to_string()
            ]
        );
        assert_eq!(t.no_key, Some(NoKey::TooFewNotes));
        assert_eq!(t.notes.len(), 3, "{:?}", t.notes);
        for (n, want) in t.notes.iter().zip(notes) {
            assert_eq!(
                (n.bar, n.step, n.steps, n.snap_error_ms),
                (None, None, None, None)
            );
            assert_eq!(n.midi, want.2);
            assert_eq!(n.name, midi_to_name(want.2));
            assert!((n.start - want.0).abs() < 0.05, "{n:?}");
            assert!(!n.tied_to_next);
            assert_eq!(n.degree, None);
        }
        // with a key the unquantized notes still get degrees
        let opts = Options {
            key: Some(parse_key_override("C:major").unwrap()),
            ..Options::default()
        };
        let t = run(&samples, 16_000, &GridHint::default(), &opts);
        assert_eq!(t.warnings, vec![UNRESOLVED_WARNING.to_string()]);
        assert_eq!(t.no_key, None);
        let degrees: Vec<_> = t.notes.iter().map(|n| n.degree.clone().unwrap()).collect();
        assert_eq!(degrees, ["0", "2", "4"]);
        assert_eq!(t.notation, None);
    }

    #[test]
    fn override_zero_disables() {
        let settings = SynthSettings {
            detune_cents: 30.0,
            ..voice()
        };
        let samples = audio(&melody(), settings);
        let estimated = run(&samples, 16_000, &grid_at(0.5), &Options::default());
        assert_eq!(estimated.tuning.source, TuningSource::Estimated);
        assert!(
            (estimated.tuning.cents - 30.0).abs() < 5.0,
            "{:?}",
            estimated.tuning
        );
        for n in &estimated.notes {
            assert!(n.cents.abs() < 8.0, "tuned out: {n:?}");
        }
        let off = Options {
            tuning_cents: Some(0.0),
            ..Options::default()
        };
        let t = run(&samples, 16_000, &grid_at(0.5), &off);
        assert_eq!(t.tuning.cents, 0.0);
        assert_eq!(t.tuning.source, TuningSource::Flag);
        for n in &t.notes {
            assert!((n.cents - 30.0).abs() < 8.0, "not tuned: {n:?}");
        }
    }

    #[test]
    fn tuning_flag_overrides_estimate() {
        // a take 45 cents flat: the estimate warns, the flag sets the value and does not
        let settings = SynthSettings {
            detune_cents: -45.0,
            ..voice()
        };
        let samples = audio(&melody(), settings);
        let t = run(&samples, 16_000, &grid_at(0.5), &Options::default());
        assert!((t.tuning.cents + 45.0).abs() < 5.0, "{:?}", t.tuning);
        let warning = format!(
            "tuning is {:.0} cents off the nearest semitone; the note names may be a semitone out. Pass --tuning",
            t.tuning.cents
        );
        assert!(t.warnings.contains(&warning), "{:?}", t.warnings);
        let opts = Options {
            tuning_cents: Some(-20.0),
            ..Options::default()
        };
        let t = run(&samples, 16_000, &grid_at(0.5), &opts);
        assert_eq!(t.tuning.cents, -20.0);
        assert_eq!(t.tuning.source, TuningSource::Flag);
        assert!(
            !t.warnings.iter().any(|w| w.starts_with("tuning is")),
            "{:?}",
            t.warnings
        );
        // -45 sung, -20 removed: the notes sit 25 cents flat of their semitones
        for n in &t.notes {
            assert!((n.cents + 25.0).abs() < 8.0, "{n:?}");
        }
    }

    #[test]
    fn key_flag_skips_detection() {
        let samples = audio(&melody(), voice());
        let detected = run(&samples, 16_000, &grid_at(0.5), &Options::default());
        let key = detected.key.as_ref().expect("detected");
        assert_eq!(key.name, "C major");
        assert_eq!(key.source, KeySource::Detected);
        assert!(key.confidence.is_some() && key.runner_up.is_some());
        let opts = Options {
            key: Some(parse_key_override("d:dorian").unwrap()),
            ..Options::default()
        };
        let t = run(&samples, 16_000, &grid_at(0.5), &opts);
        let key = t.key.as_ref().expect("flag");
        assert_eq!(key.name, "D dorian");
        assert_eq!(key.source, KeySource::Flag);
        assert_eq!(
            (key.confidence, key.runner_up.clone(), key.correlation),
            (None, None, None)
        );
        let notation = t.notation.as_ref().unwrap();
        // median d4 (sorted 60 60 62 62 64 64 65 67: the lower middle), root D4
        assert_eq!(notation.scale.as_deref(), Some("D4:dorian"));
        assert_eq!(notation.note, detected.notation.as_ref().unwrap().note);
        assert_eq!(
            t.notes[0].degree.as_deref(),
            Some("-1"),
            "c4 below D4 in dorian"
        );
        // a melody too short to detect a key gets one from the flag, without the warning
        let short = audio(&melody()[..3], voice());
        let t = run(&short, 16_000, &grid_at(0.5), &opts);
        assert!(t.key.is_some());
        assert!(
            !t.warnings.iter().any(|w| w.starts_with("no key")),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn debug_fills_frames_and_onsets_only_when_set() {
        let samples = audio(&melody(), voice());
        let t = run(&samples, 16_000, &grid_at(0.5), &Options::default());
        assert!(t.frames.is_none() && t.onsets.is_none());
        let opts = Options {
            debug: true,
            span_start_seconds: 3.0,
            ..Options::default()
        };
        let t = run(&samples, 16_000, &grid_at(3.5), &opts);
        let frames = t.frames.as_ref().expect("frames");
        assert_eq!(
            frames.len(),
            PyinTracker.track(&samples, 16_000).frames.len()
        );
        assert_eq!(frames[0].t, 0.0, "span time");
        assert!((frames[1].t - 0.01).abs() < 1e-9);
        assert!(frames.iter().any(|f| f.f0.is_some()));
        let onsets = t.onsets.as_ref().expect("onsets");
        assert_eq!(onsets, &onset_strength(&samples).peaks);
        assert!(
            onsets.iter().all(|o| o.seconds < 4.0),
            "span time, not memo time"
        );
        let v = serde_json::to_value(&t).unwrap();
        assert!(v["frames"].is_array() && v["onsets"].is_array());
    }

    #[test]
    fn span_offset_is_added_to_note_times() {
        let samples = audio(&melody(), voice());
        let whole = run(&samples, 16_000, &grid_at(0.5), &Options::default());
        let opts = Options {
            span_start_seconds: 10.0,
            memo_seconds: Some(60.0),
            ..Options::default()
        };
        let span = run(&samples, 16_000, &grid_at(10.5), &opts);
        assert_eq!(whole.notes.len(), span.notes.len());
        for (a, b) in whole.notes.iter().zip(&span.notes) {
            assert!((b.start - a.start - 10.0).abs() < 1e-9, "{a:?} {b:?}");
            assert!((b.end - a.end - 10.0).abs() < 1e-9);
            assert_eq!((a.bar, a.step, a.steps), (b.bar, b.step, b.steps));
        }
        assert_eq!(whole.notation, span.notation);
        assert_eq!(span.grid.as_ref().unwrap().origin, 10.5);
        assert_eq!(
            span.duration, whole.duration,
            "seconds analysed, not memo time"
        );
        // memo_seconds bounds bar 1: past it is Bar1PastEnd, past the span alone is not
        let err = transcribe(&samples, 16_000, &grid_at(61.0), &opts).unwrap_err();
        assert!(matches!(err, TranscribeError::Bar1PastEnd { .. }), "{err}");
        assert!(transcribe(&samples, 16_000, &grid_at(30.0), &opts).is_ok());
        // without memo_seconds the memo ends with the samples
        let no_len = Options {
            memo_seconds: None,
            ..opts
        };
        let end = 10.0 + samples.len() as f64 / 16_000.0;
        match transcribe(&samples, 16_000, &grid_at(end + 0.01), &no_len).unwrap_err() {
            TranscribeError::Bar1PastEnd { len, .. } => assert!((len - end).abs() < 1e-9),
            other => panic!("{other}"),
        }
    }

    #[test]
    fn hint_meter_names_the_grid_and_invalid_hints_are_absent() {
        let samples = audio(&melody(), voice());
        let plain = run(&samples, 16_000, &grid_at(0.5), &Options::default());
        assert_eq!(plain.grid.as_ref().unwrap().meter, "4/4");
        // a meter as written: 2/2 is four quarter notes, reported as given
        let hint = GridHint {
            meter: Some("2/2".into()),
            ..grid_at(0.5)
        };
        let t = run(&samples, 16_000, &hint, &Options::default());
        let grid = t.grid.as_ref().unwrap();
        assert_eq!((grid.meter.as_str(), grid.beats_per_bar), ("2/2", 4.0));
        assert_eq!(t.notes, plain.notes);
        // unusable values count as absent: the grid is inferred, the meter assumed
        let bad = GridHint {
            bpm: Some(Hinted::flag(f64::NAN)),
            beats_per_bar: Some(Hinted::sidecar(0.0)),
            origin_seconds: Some(Hinted::flag(f64::INFINITY)),
            meter: Some("2/2".into()),
            tempo_unheard: false,
        };
        let t = run(&samples, 16_000, &bad, &Options::default());
        let none = run(&samples, 16_000, &GridHint::default(), &Options::default());
        assert_eq!(t, none);
    }

    #[test]
    fn too_short_take_is_ok_and_empty() {
        for (samples, rate) in [(vec![0.3f32; 1023], 16_000u32), (vec![0.3; 3000], 48_000)] {
            let t = run(&samples, rate, &grid_at(0.0), &Options::default());
            assert_eq!(t.warnings, vec![TOO_SHORT_WARNING.to_string()], "{rate}");
            assert!(t.notes.is_empty() && t.notation.is_none() && t.grid.is_none());
            assert_eq!(t.key, None);
            assert_eq!(t.duration, samples.len() as f64 / f64::from(rate));
        }
        assert!(
            run(&[], 16_000, &grid_at(0.0), &Options::default())
                .notes
                .is_empty()
        );
        // one frame exactly is analysed
        let t = run(&[0.0; 1024], 16_000, &grid_at(0.0), &Options::default());
        assert!(
            !t.warnings.contains(&TOO_SHORT_WARNING.to_string()),
            "{:?}",
            t.warnings
        );
        assert!(
            t.warnings.contains(&NO_NOTES_WARNING.to_string()),
            "{:?}",
            t.warnings
        );
        assert_eq!(t.notation, None);
        assert_eq!(t.timing_spread, None);
    }

    #[test]
    fn suggestions_name_a_better_grid_and_leave_the_notes_alone() {
        // dotted eighths at 120 bpm (0.375 s apart) from bar 1, alternating pitches
        let notes: Vec<(f64, f64, i32)> = (0..12)
            .map(|k| {
                let t = 1.0 + 0.375 * k as f64;
                (t, t + 0.3, if k % 2 == 0 { 60 } else { 64 })
            })
            .collect();
        let samples = audio(&notes, voice());
        let eighths = Options {
            grid: GridDivision::Eighth,
            ..Options::default()
        };
        let t = run(&samples, 16_000, &grid_at(1.0), &eighths);
        let grid = t.grid.as_ref().expect("grid");
        assert_eq!(grid.suggested_division, Some(GridDivision::Sixteenth));
        assert_eq!(grid.division, GridDivision::Eighth, "only a suggestion");
        assert!(
            t.warnings
                .iter()
                .any(|w| w.starts_with("timing fits --grid 16 better")),
            "{:?}",
            t.warnings
        );
        assert_eq!(grid.suggested_shift_ms, None, "on the grid");
        // on sixteenths nothing is suggested
        let t = run(&samples, 16_000, &grid_at(1.0), &Options::default());
        let grid = t.grid.as_ref().expect("grid");
        assert_eq!(
            (grid.suggested_division, grid.suggested_shift_ms),
            (None, None)
        );
        // the grid 60 ms early: the notes sit after it, a shift is suggested
        let t = run(&samples, 16_000, &grid_at(0.94), &Options::default());
        let shift = t
            .grid
            .as_ref()
            .and_then(|g| g.suggested_shift_ms)
            .expect("a shift");
        assert!((shift - 60.0).abs() <= 15.0, "{shift}");
        assert!(
            t.warnings
                .iter()
                .any(|w| w.contains("ms after the grid; try --shift")),
            "{:?}",
            t.warnings
        );
        // eighths on that grid: the shift is measured on the suggested sixteenths, so the two
        // go together
        let t = run(&samples, 16_000, &grid_at(0.94), &eighths);
        let grid = t.grid.as_ref().expect("grid");
        assert_eq!(grid.suggested_division, Some(GridDivision::Sixteenth));
        let shift = grid.suggested_shift_ms.expect("a shift on sixteenths");
        assert!((shift - 60.0).abs() <= 15.0, "{shift}");
        assert!(
            t.warnings.iter().any(|w| w.ends_with(&format!(
                "ms after the --grid 16 grid; try --grid 16 --shift {shift:.0}"
            ))),
            "{:?}",
            t.warnings
        );
    }

    #[test]
    fn a_pickup_bar_starts_the_notation() {
        // the grid starts at 1.0 s: the first note (0.5 to 1.0) is a pickup, bar 0
        let samples = audio(&melody(), voice());
        let t = run(&samples, 16_000, &grid_at(1.0), &Options::default());
        assert_eq!(t.notes[0].bar, Some(0));
        assert_eq!(t.notes[0].midi, 60);
        assert!(
            !t.warnings.iter().any(|w| w.contains("before bar 1")),
            "{:?}",
            t.warnings
        );
        let notation = t.notation.as_ref().expect("notation");
        // bar 0 is the first cycle, and the last c4 sustains into bar 2
        assert_eq!(
            notation.note, "[~@6 c4@2 d4 e4 f4 g4 e4 d4 c4@4 ~@6]/3",
            "{notation:?}"
        );
        assert_eq!((notation.first_bar, notation.last_bar), (0, 2));
        // a span that starts at bar 1 does not show the pickup
        let opts = Options {
            span_start_seconds: 1.0,
            ..Options::default()
        };
        let t = run(&samples[16_000..], 16_000, &grid_at(1.0), &opts);
        assert_eq!(t.notation.as_ref().map(|n| n.first_bar), Some(1));
    }

    #[test]
    fn a_bars_request_from_bar_1_leaves_the_pickup_in_the_notes() {
        // the a_pickup_bar_starts_the_notation take: the c4 at 0.5 s is in bar 0
        let samples = audio(&melody(), voice());
        let whole = run(&samples, 16_000, &grid_at(1.0), &Options::default());
        assert_eq!(whole.notation.as_ref().map(|n| n.first_bar), Some(0));
        let opts = Options {
            bars: Some((1, 2)),
            ..Options::default()
        };
        let t = run(&samples, 16_000, &grid_at(1.0), &opts);
        let notation = t.notation.as_ref().expect("notation");
        assert_eq!((notation.first_bar, notation.last_bar), (1, 2));
        assert_eq!(t.notes, whole.notes);
        assert!(t.notes.iter().any(|n| n.bar == Some(0)), "{:?}", t.notes);
    }

    #[test]
    fn bad_bars_requests_are_bad_options() {
        for (bars, want) in [
            ((0, 1), "the bars must be 1 <= A <= B (got 0-1)"),
            ((3, 2), "the bars must be 1 <= A <= B (got 3-2)"),
        ] {
            let opts = Options {
                bars: Some(bars),
                ..Options::default()
            };
            match transcribe(&[0.0; 4096], 16_000, &grid_at(0.0), &opts) {
                Err(TranscribeError::BadOption(m)) => assert_eq!(m, want),
                other => panic!("{bars:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn notes_more_than_a_bar_before_origin_warn_and_stay_out_of_notation() {
        // origin 2.6 s, a bar of 2 s: the c4 at 0.5 s is more than a bar early
        let samples = audio(&melody(), voice());
        let t = run(&samples, 16_000, &grid_at(2.6), &Options::default());
        assert_eq!((t.notes[0].bar, t.notes[0].midi), (Some(0), 60));
        assert!(
            t.warnings.contains(
                &"1 note more than a bar before bar 1 is in the JSON (bar 0) and not in the notation"
                    .to_string()
            ),
            "{:?}",
            t.warnings
        );
        let notation = t.notation.as_ref().expect("notation");
        assert_eq!(notation.first_bar, 0);
        assert!(!notation.note.contains("c4 d4"), "{notation:?}");
        // every note before bar 1: the pickup bar alone, without the notes before it
        let t = run(&samples, 16_000, &grid_at(3.6), &Options::default());
        assert!(t.notes.iter().all(|n| n.bar == Some(0)), "{:?}", t.notes);
        let notation = t.notation.as_ref().expect("notation");
        assert_eq!((notation.first_bar, notation.last_bar), (0, 0));
        assert_eq!(notation.note, "~ g4@2 e4@2 d4@2 c4@8 ~", "{notation:?}");
    }

    #[test]
    fn a_take_only_in_the_pickup_bar_is_notated_as_bar_0() {
        // origin 4.0 s, a bar of 2 s: every note is in the pickup bar (2.0 to 4.0)
        let notes = [(2.5, 3.0, 60), (3.0, 3.5, 64), (3.5, 4.0, 67)];
        let samples = audio(&notes, voice());
        let t = run(&samples, 16_000, &grid_at(4.0), &Options::default());
        assert!(t.notes.iter().all(|n| n.bar == Some(0)), "{:?}", t.notes);
        let notation = t.notation.as_ref().expect("notation");
        assert_eq!((notation.first_bar, notation.last_bar), (0, 0));
        assert_eq!(notation.note, "~ c4 e4 g4", "{notation:?}");
    }
}
