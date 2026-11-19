//! Notes snapped to the grid.
//!
//! A step is `60 / bpm / steps_per_quarter` seconds and the steps are counted from the grid's
//! origin (step 0 is bar 1's downbeat). Per note, in order:
//!
//! 1. Its onset snaps to the nearest step, its offset to the nearest step after the onset, so
//!    every note is at least one step long.
//! 2. Notes that snap to the same onset step keep the longest by raw duration (the earliest on a
//!    tie); the others go to `dropped` with [`SAME_STEP_REASON`].
//! 3. An overlap cuts the earlier note at the later note's onset.
//! 4. A note that crosses a barline at or after the origin is split there and the rest
//!    re-attacked (Strudel's mini-notation has no tie across cycles): every part but the last has
//!    `tied_to_next`, a continuation starts at the barline's time with a snap error of 0, and a
//!    warning names the bar it crosses into. Notes before the origin are bar 0, their `step`
//!    counted within their own bar, and are not split at earlier barlines. Those that start in
//!    the bar just before the origin are the pickup bar, which the notation renders as its first
//!    cycle; those that start earlier are `early`, left out of the notation with a warning.
//!
//! The snap error is `quantized start - raw start` in milliseconds, signed, and the confidence is
//! multiplied by `1 - min(|snap error|, SNAP_PENALTY_MS) / 100` (`components` stay without it).
//! The timing spread is the nearest-rank 90th percentile of `|snap error|` over the attacks (the
//! continuations of a split note are not attacks): the value at rank `ceil(0.9 * n)` of the
//! sorted errors, 0 without notes.
//!
//! Raw note times must be memo seconds, like the grid's origin: the caller adds the span's start
//! to [`RawNote`] times before quantizing. Note `start` and `end` are copied from them.

use serde::{Deserialize, Serialize};

use crate::consts::{SNAP_PENALTY_MS, TIMING_SPREAD_WARN_MS};
use crate::error::TranscribeError;
use crate::grid::{Grid, meter_label};
use crate::notation::midi_to_name;
use crate::report::{Alternative, Dropped, Note};
use crate::segment::RawNote;

/// The quantizing grid's steps per quarter note (`--grid`); serialized as its `--grid` spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GridDivision {
    /// `4`: quarter notes.
    #[serde(rename = "4")]
    Quarter,
    /// `8`: eighth notes.
    #[serde(rename = "8")]
    Eighth,
    /// `16`: sixteenth notes (the default).
    #[default]
    #[serde(rename = "16")]
    Sixteenth,
    /// `8t`: eighth-note triplets.
    #[serde(rename = "8t")]
    EighthTriplet,
    /// `16t`: sixteenth-note triplets.
    #[serde(rename = "16t")]
    SixteenthTriplet,
}

impl GridDivision {
    /// Steps per quarter note: 1, 2, 4, 3 and 6.
    pub fn steps_per_quarter(self) -> u32 {
        match self {
            Self::Quarter => 1,
            Self::Eighth => 2,
            Self::Sixteenth => 4,
            Self::EighthTriplet => 3,
            Self::SixteenthTriplet => 6,
        }
    }

    /// The `--grid` value: exactly `4`, `8`, `16`, `8t` or `16t`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "4" => Some(Self::Quarter),
            "8" => Some(Self::Eighth),
            "16" => Some(Self::Sixteenth),
            "8t" => Some(Self::EighthTriplet),
            "16t" => Some(Self::SixteenthTriplet),
            _ => None,
        }
    }

    /// The `--grid` spelling, as [`GridDivision::parse`] reads it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Quarter => "4",
            Self::Eighth => "8",
            Self::Sixteenth => "16",
            Self::EighthTriplet => "8t",
            Self::SixteenthTriplet => "16t",
        }
    }
}

/// Notes on the grid, the notes it removed, and what it noticed.
#[derive(Debug, Clone, PartialEq)]
pub struct Quantized {
    pub notes: Vec<Note>,
    pub dropped: Vec<Dropped>,
    pub steps_per_bar: u32,
    /// Milliseconds, the nearest-rank 90th percentile of `|snap error|` over the attacks.
    pub timing_spread_ms: f32,
    pub warnings: Vec<String>,
}

/// The `Dropped.reason` of a note that lost its onset step to a longer one.
pub const SAME_STEP_REASON: &str = "starts on the same grid step as a longer note";

/// A note longer than this many bars is cut, so a nonsense duration cannot split forever.
const MAX_NOTE_BARS: i64 = 100_000;

/// Steps in a bar of `beats_per_bar` quarter notes at `division`; `GridDoesNotDivide` when that
/// is not a whole positive number that fits a `u32`.
pub fn steps_per_bar(beats_per_bar: f64, division: GridDivision) -> Result<u32, TranscribeError> {
    let steps = beats_per_bar * f64::from(division.steps_per_quarter());
    let rounded = steps.round();
    if steps.is_finite()
        && rounded >= 1.0
        && (steps - rounded).abs() < 1e-9
        && rounded <= f64::from(u32::MAX)
    {
        return Ok(rounded as u32);
    }
    Err(TranscribeError::GridDoesNotDivide {
        division: division.label().to_string(),
        meter: meter_label(beats_per_bar),
        steps,
    })
}

/// A raw note on the grid while collisions are resolved.
struct Slot<'a> {
    raw: &'a RawNote,
    onset: i64,
    end: i64,
}

/// Snaps `raw` (memo seconds) to `grid` at `division`. The grid is a resolved one: a positive
/// finite tempo and a finite origin.
pub fn quantize(
    raw: &[RawNote],
    grid: &Grid,
    division: GridDivision,
) -> Result<Quantized, TranscribeError> {
    let spb = steps_per_bar(grid.beats_per_bar, division)?;
    let step_s = 60.0 / grid.bpm / f64::from(division.steps_per_quarter());
    let origin = grid.origin_seconds;
    let bar_steps = i64::from(spb);
    let to_step = |t: f64| ((t - origin) / step_s).round() as i64;

    let mut slots: Vec<Slot> = raw
        .iter()
        .filter(|n| n.start.is_finite() && n.end.is_finite() && n.midi.is_finite())
        .map(|n| {
            let onset = to_step(n.start);
            let end = to_step(n.end.max(n.start))
                .max(onset.saturating_add(1))
                .min(onset.saturating_add(MAX_NOTE_BARS * bar_steps));
            Slot { raw: n, onset, end }
        })
        .collect();
    slots.sort_by(|a, b| {
        a.onset
            .cmp(&b.onset)
            .then(a.raw.start.total_cmp(&b.raw.start))
    });

    // same onset step: keep the longest raw duration, the earliest on a tie
    let mut kept: Vec<Slot> = Vec::with_capacity(slots.len());
    let mut dropped = Vec::new();
    for slot in slots {
        match kept.last_mut() {
            Some(last) if last.onset == slot.onset => {
                let duration = |n: &RawNote| n.end - n.start;
                let loser = if duration(slot.raw) > duration(last.raw) {
                    std::mem::replace(last, slot)
                } else {
                    slot
                };
                dropped.push(dropped_of(loser.raw, SAME_STEP_REASON));
            }
            _ => kept.push(slot),
        }
    }
    dropped.sort_by(|a, b| a.start.total_cmp(&b.start));

    // overlaps cut the earlier note
    for i in 1..kept.len() {
        let next = kept[i].onset;
        let prev = &mut kept[i - 1];
        prev.end = prev.end.min(next);
    }

    let mut notes = Vec::with_capacity(kept.len());
    let mut warnings = Vec::new();
    let mut snaps = Vec::with_capacity(kept.len());
    let mut before_origin = 0usize;
    for slot in &kept {
        let n = slot.raw;
        let quantized_start = origin + slot.onset as f64 * step_s;
        let snap = ((quantized_start - n.start) * 1000.0) as f32;
        snaps.push(snap.abs());
        let confidence = n.confidence * (1.0 - snap.abs().min(SNAP_PENALTY_MS) / 100.0);
        let rounded = n.midi.round();
        let midi = rounded as i32;
        let cents = (n.midi - rounded) * 100.0;
        let is_early = slot.onset < -bar_steps;
        if is_early {
            before_origin += 1;
        }

        let mut from = slot.onset;
        while from < slot.end {
            let barline = if from < 0 {
                0
            } else {
                from.div_euclid(bar_steps)
                    .saturating_add(1)
                    .saturating_mul(bar_steps)
            };
            let to = slot.end.min(barline);
            let last = to == slot.end;
            let first = from == slot.onset;
            let bar = if from < 0 {
                0
            } else {
                from.div_euclid(bar_steps).saturating_add(1)
            };
            notes.push(Note {
                start: if first {
                    n.start
                } else {
                    origin + from as f64 * step_s
                },
                end: if last {
                    n.end
                } else {
                    origin + to as f64 * step_s
                },
                bar: Some(u32::try_from(bar).unwrap_or(u32::MAX)),
                step: Some(u32::try_from(from.rem_euclid(bar_steps)).unwrap_or(u32::MAX)),
                steps: Some(u32::try_from(to - from).unwrap_or(u32::MAX)),
                midi,
                name: midi_to_name(midi),
                degree: None,
                cents,
                confidence,
                components: n.components.clone(),
                snap_error_ms: Some(if first { snap } else { 0.0 }),
                tied_to_next: !last,
                alternative: Alternative::of(midi, cents),
                early: is_early && from < 0,
            });
            if !last {
                let into = if to <= 0 {
                    1
                } else {
                    to.div_euclid(bar_steps).saturating_add(1)
                };
                warnings.push(format!(
                    "a note crosses the barline into bar {into}; it is split and re-attacked"
                ));
            }
            from = to;
        }
    }
    if before_origin > 0 {
        warnings.push(before_origin_warning(before_origin));
    }
    let timing_spread_ms = p90(&mut snaps);
    if timing_spread_ms > TIMING_SPREAD_WARN_MS {
        warnings.push(format!(
            "timing spread is {timing_spread_ms:.0} ms; the grid or the phase may be wrong (try --shift or --bar1)"
        ));
    }

    Ok(Quantized {
        notes,
        dropped,
        steps_per_bar: spb,
        timing_spread_ms,
        warnings,
    })
}

/// The warning for `count` notes that start more than a bar before the origin: `1 note more
/// than a bar before bar 1 is ...`, `N notes more than a bar before bar 1 are ...`.
pub fn before_origin_warning(count: usize) -> String {
    if count == 1 {
        "1 note more than a bar before bar 1 is in the JSON (bar 0) and not in the notation"
            .to_string()
    } else {
        format!(
            "{count} notes more than a bar before bar 1 are in the JSON (bar 0) and not in the notation"
        )
    }
}

/// The nearest-rank 90th percentile: the value at rank `ceil(0.9 * n)` of the sorted values;
/// 0 when empty.
fn p90(values: &mut [f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f32::total_cmp);
    let rank = (values.len() * 9).div_ceil(10).max(1);
    values[rank - 1]
}

fn dropped_of(n: &RawNote, reason: &str) -> Dropped {
    let midi = n.midi.round() as i32;
    Dropped {
        start: n.start,
        end: n.end,
        midi,
        name: midi_to_name(midi),
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Resolved;
    use crate::report::Components;

    fn components() -> Components {
        Components {
            voiced: 0.9,
            stability: 0.8,
            fit: 0.7,
            onset: None,
        }
    }

    fn raw(start: f64, end: f64, midi: f32) -> RawNote {
        RawNote {
            start,
            end,
            midi,
            components: components(),
            confidence: 0.8,
            split_by_onset: false,
        }
    }

    /// 120 bpm 4/4 from `origin`: a sixteenth is 0.125 s, a bar 2 s.
    fn grid(origin: f64) -> Grid {
        Grid {
            bpm: 120.0,
            beats_per_bar: 4.0,
            origin_seconds: origin,
            tempo_source: Resolved::Sidecar,
            phase_source: Resolved::Sidecar,
            confidence: None,
        }
    }

    /// `(bar, step, steps)` of every note.
    fn positions(q: &Quantized) -> Vec<(u32, u32, u32)> {
        q.notes
            .iter()
            .map(|n| (n.bar.unwrap(), n.step.unwrap(), n.steps.unwrap()))
            .collect()
    }

    #[test]
    fn steps_per_bar_for_every_division() {
        use GridDivision::*;
        // (beats per bar, meter, [4, 8, 16, 8t, 16t])
        let table: [(f64, &str, [u32; 5]); 4] = [
            (4.0, "4/4", [4, 8, 16, 12, 24]),
            (3.0, "3/4", [3, 6, 12, 9, 18]),
            (3.0, "6/8", [3, 6, 12, 9, 18]),
            (5.0, "5/4", [5, 10, 20, 15, 30]),
        ];
        let divisions = [Quarter, Eighth, Sixteenth, EighthTriplet, SixteenthTriplet];
        for (beats, meter, expected) in table {
            for (division, want) in divisions.iter().zip(expected) {
                assert_eq!(
                    steps_per_bar(beats, *division).unwrap(),
                    want,
                    "{meter} at {}",
                    division.label()
                );
            }
        }
        // 3/8 divides at eighths, sixteenths and sixteenth triplets only
        assert_eq!(steps_per_bar(1.5, Eighth).unwrap(), 3);
        assert_eq!(steps_per_bar(1.5, Sixteenth).unwrap(), 6);
        assert_eq!(steps_per_bar(1.5, SixteenthTriplet).unwrap(), 9);
        assert_eq!(
            steps_per_bar(1.5, EighthTriplet).unwrap_err().to_string(),
            "--grid 8t does not divide a 3/8 bar into whole steps (4.5 steps); try a finer grid"
        );
    }

    #[test]
    fn non_integer_steps_is_error() {
        let err = steps_per_bar(1.5, GridDivision::Quarter).unwrap_err();
        assert_eq!(
            err.to_string(),
            "--grid 4 does not divide a 3/8 bar into whole steps (1.5 steps); try a finer grid"
        );
        assert_eq!(err.exit_code(), 4);
        let g = Grid {
            beats_per_bar: 1.5,
            ..grid(0.0)
        };
        let err = quantize(&[raw(0.0, 1.0, 60.0)], &g, GridDivision::Quarter).unwrap_err();
        assert!(
            matches!(err, TranscribeError::GridDoesNotDivide { .. }),
            "{err:?}"
        );
        // 7/16 at sixteenth triplets is 10.5 steps
        let err = steps_per_bar(1.75, GridDivision::SixteenthTriplet).unwrap_err();
        assert_eq!(
            err.to_string(),
            "--grid 16t does not divide a 7/16 bar into whole steps (10.5 steps); try a finer grid"
        );
        for beats in [0.0, -4.0, f64::NAN, f64::INFINITY] {
            assert!(
                steps_per_bar(beats, GridDivision::Sixteenth).is_err(),
                "{beats}"
            );
        }
    }

    #[test]
    fn onset_snaps_to_nearest_step() {
        // origin 1.0; steps at 1.0, 1.125, 1.25, ...
        let notes = [
            raw(1.06, 1.24, 60.0),  // 0.48 step: step 0
            raw(1.32, 1.49, 62.0),  // 2.56 steps: step 3
            raw(1.555, 1.99, 64.0), // 4.44 steps: step 4
        ];
        let q = quantize(&notes, &grid(1.0), GridDivision::Sixteenth).unwrap();
        assert_eq!(positions(&q), vec![(1, 0, 2), (1, 3, 1), (1, 4, 4)]);
        assert_eq!(q.steps_per_bar, 16);
        assert!(q.dropped.is_empty());
        let names: Vec<&str> = q.notes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["c4", "d4", "e4"]);
        // raw times are kept
        assert_eq!(q.notes[1].start, 1.32);
        assert_eq!(q.notes[1].end, 1.49);
        // at the eighth grid the same notes land on eighths
        let q = quantize(&notes, &grid(1.0), GridDivision::Eighth).unwrap();
        assert_eq!(positions(&q), vec![(1, 0, 1), (1, 1, 1), (1, 2, 2)]);
    }

    #[test]
    fn every_note_at_least_one_step() {
        let notes = [
            raw(1.0, 1.02, 60.0),
            raw(1.5, 1.5, 62.0),
            raw(2.0, 1.9, 64.0),
        ];
        let q = quantize(&notes, &grid(1.0), GridDivision::Sixteenth).unwrap();
        assert_eq!(positions(&q), vec![(1, 0, 1), (1, 4, 1), (1, 8, 1)]);
        let q = quantize(&notes, &grid(1.0), GridDivision::Quarter).unwrap();
        assert_eq!(positions(&q), vec![(1, 0, 1), (1, 1, 1), (1, 2, 1)]);
    }

    #[test]
    fn overlap_cuts_earlier_note() {
        // the first note's offset snaps to step 6, past the second's onset at step 4
        let notes = [raw(0.0, 0.74, 60.0), raw(0.49, 1.0, 62.0)];
        let q = quantize(&notes, &grid(0.0), GridDivision::Sixteenth).unwrap();
        assert_eq!(positions(&q), vec![(1, 0, 4), (1, 4, 4)]);
        assert!(!q.notes[0].tied_to_next);
        assert_eq!(q.notes[0].end, 0.74, "raw end kept");
    }

    #[test]
    fn same_onset_keeps_longer_and_reports_dropped() {
        // both snap to step 2; the second is longer by raw duration though less confident
        let mut grace = raw(0.24, 0.30, 61.0);
        grace.confidence = 0.95;
        let mut held = raw(0.27, 0.74, 64.0);
        held.confidence = 0.4;
        let q = quantize(&[grace, held], &grid(0.0), GridDivision::Sixteenth).unwrap();
        assert_eq!(positions(&q), vec![(1, 2, 4)]);
        assert_eq!(q.notes[0].midi, 64);
        assert_eq!(
            q.dropped,
            vec![Dropped {
                start: 0.24,
                end: 0.30,
                midi: 61,
                name: "c#4".to_string(),
                reason: "starts on the same grid step as a longer note".to_string(),
            }]
        );
        // equal durations keep the earlier
        let q = quantize(
            &[
                raw(0.2578125, 0.5078125, 60.0),
                raw(0.2421875, 0.4921875, 62.0),
            ],
            &grid(0.0),
            GridDivision::Sixteenth,
        )
        .unwrap();
        assert_eq!(q.notes[0].midi, 62);
        assert_eq!(q.dropped.len(), 1);
        assert_eq!(q.dropped[0].midi, 60);
    }

    #[test]
    fn barline_crossing_splits_and_ties() {
        // origin 0.5, bar 2 starts at 2.5 (step 16); the note runs steps 14..20
        let notes = [raw(2.25, 3.0, 67.0), raw(3.0, 3.5, 69.0)];
        let q = quantize(&notes, &grid(0.5), GridDivision::Sixteenth).unwrap();
        assert_eq!(positions(&q), vec![(1, 14, 2), (2, 0, 4), (2, 4, 4)]);
        let (a, b) = (&q.notes[0], &q.notes[1]);
        assert!(a.tied_to_next);
        assert!(!b.tied_to_next);
        assert!(!q.notes[2].tied_to_next);
        assert_eq!((a.start, a.end), (2.25, 2.5));
        assert_eq!((b.start, b.end), (2.5, 3.0));
        assert_eq!((a.midi, b.midi), (67, 67));
        assert_eq!(b.snap_error_ms, Some(0.0));
        assert_eq!(
            q.warnings,
            vec!["a note crosses the barline into bar 2; it is split and re-attacked".to_string()]
        );
        // a note over two barlines splits in three
        let q = quantize(&[raw(1.5, 5.0, 60.0)], &grid(0.5), GridDivision::Sixteenth).unwrap();
        assert_eq!(positions(&q), vec![(1, 8, 8), (2, 0, 16), (3, 0, 4)]);
        let ties: Vec<bool> = q.notes.iter().map(|n| n.tied_to_next).collect();
        assert_eq!(ties, [true, true, false]);
        assert_eq!(q.warnings.len(), 2);
        assert!(q.warnings[1].contains("into bar 3;"));
        // the attack's snap error alone counts toward the spread
        assert_eq!(q.timing_spread_ms, 0.0);
    }

    fn early(q: &Quantized) -> Vec<bool> {
        q.notes.iter().map(|n| n.early).collect()
    }

    #[test]
    fn notes_before_origin_are_bar_0() {
        // origin 2.0: a pickup a beat early, one far earlier in the bar, one across the origin
        let notes = [
            raw(0.10, 0.30, 55.0),
            raw(1.50, 1.70, 57.0),
            raw(1.75, 2.25, 59.0),
            raw(2.5, 2.75, 60.0),
        ];
        let q = quantize(&notes, &grid(2.0), GridDivision::Sixteenth).unwrap();
        // steps: 0.10 -> -15.2 -> -15 (step 1 of its bar), its end -13.6 -> -14; 1.50 -> -4 (step 12)
        assert_eq!(
            positions(&q),
            vec![(0, 1, 1), (0, 12, 2), (0, 14, 2), (1, 0, 2), (1, 4, 2)]
        );
        for n in &q.notes {
            assert!(n.step.is_some() && n.steps.is_some());
        }
        assert!(q.notes[2].tied_to_next);
        // all three are in the pickup bar (a bar is 2 s): none is early, none warns
        assert_eq!(early(&q), vec![false; 5]);
        assert_eq!(
            q.warnings,
            vec!["a note crosses the barline into bar 1; it is split and re-attacked".to_string()]
        );
    }

    #[test]
    fn notes_in_the_pickup_bar_do_not_warn() {
        // a pickup a beat early: bar 0, in the notation, no warning
        let notes = [raw(1.50, 1.70, 57.0), raw(2.5, 2.75, 60.0)];
        let q = quantize(&notes, &grid(2.0), GridDivision::Sixteenth).unwrap();
        assert!(q.warnings.is_empty(), "{:?}", q.warnings);
        assert_eq!(early(&q), vec![false, false]);
        // the pickup bar's first step is still in it; a step earlier is not (a bar is 2 s)
        let notes = [raw(0.0, 0.2, 57.0), raw(2.5, 2.75, 60.0)];
        let q = quantize(&notes, &grid(2.0), GridDivision::Sixteenth).unwrap();
        assert_eq!(early(&q), vec![false, false]);
        let notes = [raw(-0.125, 0.2, 57.0), raw(2.5, 2.75, 60.0)];
        let q = quantize(&notes, &grid(2.0), GridDivision::Sixteenth).unwrap();
        assert_eq!(early(&q), vec![true, false]);
        assert_eq!(q.warnings, vec![before_origin_warning(1)]);
        assert_eq!(
            before_origin_warning(2),
            "2 notes more than a bar before bar 1 are in the JSON (bar 0) and not in the notation"
        );
    }

    #[test]
    fn snap_error_signed_ms() {
        // late by 20 ms: negative; early by 30 ms: positive
        let notes = [raw(1.02, 1.2, 60.0), raw(1.22, 1.4, 62.0)];
        let q = quantize(&notes, &grid(1.0), GridDivision::Sixteenth).unwrap();
        let snaps: Vec<f32> = q.notes.iter().map(|n| n.snap_error_ms.unwrap()).collect();
        assert!((snaps[0] + 20.0).abs() < 1e-3, "{snaps:?}");
        assert!((snaps[1] - 30.0).abs() < 1e-3, "{snaps:?}");
    }

    #[test]
    fn large_snap_error_lowers_confidence() {
        // on the grid: unchanged; 20 ms off: x0.8; 60 ms off: capped at 50 ms, x0.5
        let notes = [
            raw(0.0, 0.2, 60.0),
            raw(0.52, 0.7, 62.0),
            raw(1.06, 1.2, 64.0),
        ];
        let q = quantize(&notes, &grid(0.0), GridDivision::Eighth).unwrap();
        let conf: Vec<f32> = q.notes.iter().map(|n| n.confidence).collect();
        assert!((conf[0] - 0.8).abs() < 1e-6, "{conf:?}");
        assert!((conf[1] - 0.64).abs() < 1e-4, "{conf:?}");
        assert!((conf[2] - 0.4).abs() < 1e-4, "{conf:?}");
        // components are reported without the penalty
        assert_eq!(q.notes[2].components, components());
    }

    #[test]
    fn timing_spread_is_p90() {
        // snap errors 1..=10 ms late: rank ceil(0.9 * 10) = 9 is 9 ms
        let notes: Vec<RawNote> = (1..=10)
            .map(|k| {
                let t = 0.5 * f64::from(k - 1) + f64::from(k) / 1000.0;
                raw(t, t + 0.2, 60.0)
            })
            .collect();
        let q = quantize(&notes, &grid(0.0), GridDivision::Sixteenth).unwrap();
        assert!(
            (q.timing_spread_ms - 9.0).abs() < 1e-3,
            "{}",
            q.timing_spread_ms
        );
        assert!(q.warnings.is_empty(), "{:?}", q.warnings);
        assert_eq!(p90(&mut [5.0]), 5.0);
        assert_eq!(p90(&mut [3.0, 1.0]), 3.0);
        assert_eq!(p90(&mut []), 0.0);
        // a spread above the threshold warns: half a quarter-note grid off
        let notes: Vec<RawNote> = (0..5)
            .map(|k| raw(0.5 * f64::from(k) + 0.2, 0.5 * f64::from(k) + 0.4, 60.0))
            .collect();
        let q = quantize(&notes, &grid(0.0), GridDivision::Quarter).unwrap();
        assert!((q.timing_spread_ms - 200.0).abs() < 1e-3);
        assert_eq!(
            q.warnings,
            vec![
                "timing spread is 200 ms; the grid or the phase may be wrong (try --shift or --bar1)"
                    .to_string()
            ]
        );
    }

    #[test]
    fn clean_synth_quantizes_to_truth() {
        use crate::consts::ANALYSIS_RATE;
        use crate::eval::synth::{Case, SynthSettings};
        use crate::pitch::PyinTracker;
        let mut case = Case::generate(1001, &SynthSettings::default());
        case.truth.retain(|n| n.bar <= 2);
        case.bars = 2;
        let samples = case.render(ANALYSIS_RATE);
        let track = PyinTracker.track(&samples, ANALYSIS_RATE);
        let tuning = crate::tuning::estimate_tuning(&track);
        let onsets = crate::onset::onset_strength(&samples);
        let notes = crate::segment::segment(&track, &onsets, tuning);
        let g = Grid {
            bpm: case.bpm,
            origin_seconds: case.origin_seconds,
            ..grid(0.0)
        };
        let q = quantize(&notes, &g, GridDivision::Sixteenth).unwrap();
        let got: Vec<(u32, u32, u32, i32)> = q
            .notes
            .iter()
            .map(|n| (n.bar.unwrap(), n.step.unwrap(), n.steps.unwrap(), n.midi))
            .collect();
        let want: Vec<(u32, u32, u32, i32)> = case
            .truth
            .iter()
            .map(|n| (n.bar, n.step, n.steps, n.midi))
            .collect();
        assert_eq!(got, want);
        assert!(q.dropped.is_empty() && q.warnings.is_empty(), "{q:?}");
        // the synth's attack makes every detected start a little late, or on time
        for n in &q.notes {
            let snap = n.snap_error_ms.unwrap();
            assert!((-25.0..=0.5).contains(&snap), "snap {snap} ms");
        }
    }

    #[test]
    fn empty_input_is_empty_output() {
        let q = quantize(&[], &grid(0.0), GridDivision::Sixteenth).unwrap();
        assert_eq!(
            q,
            Quantized {
                notes: vec![],
                dropped: vec![],
                steps_per_bar: 16,
                timing_spread_ms: 0.0,
                warnings: vec![],
            }
        );
        // non-finite notes are skipped
        let q = quantize(
            &[raw(f64::NAN, 1.0, 60.0), raw(0.0, 1.0, f32::NAN)],
            &grid(0.0),
            GridDivision::Sixteenth,
        )
        .unwrap();
        assert!(q.notes.is_empty());
    }

    #[test]
    fn extreme_values_do_not_panic() {
        let notes = [
            raw(-1e300, 1e300, 60.0),
            raw(1e18, 1e19, 1e9),
            raw(0.0, 1.0, -1e9),
        ];
        for g in [
            grid(0.0),
            Grid {
                bpm: 1e300,
                ..grid(0.0)
            },
            Grid {
                bpm: 1e-300,
                ..grid(0.0)
            },
            grid(-1e300),
        ] {
            for division in [GridDivision::Quarter, GridDivision::SixteenthTriplet] {
                let q = quantize(&notes, &g, division).unwrap();
                assert!(!q.timing_spread_ms.is_nan(), "{q:?}");
            }
        }
    }

    #[test]
    fn pitch_rounds_and_keeps_cents() {
        let q = quantize(
            &[raw(0.0, 0.5, 60.3), raw(0.5, 1.0, 61.7)],
            &grid(0.0),
            GridDivision::Sixteenth,
        )
        .unwrap();
        assert_eq!((q.notes[0].midi, q.notes[1].midi), (60, 62));
        assert!((q.notes[0].cents - 30.0).abs() < 1e-3);
        assert!((q.notes[1].cents + 30.0).abs() < 1e-3);
        assert_eq!(q.notes[1].name, "d4");
        assert_eq!(q.notes[0].degree, None);
    }

    #[test]
    fn division_parse_cases() {
        use GridDivision::*;
        for (s, d, spq) in [
            ("4", Quarter, 1),
            ("8", Eighth, 2),
            ("16", Sixteenth, 4),
            ("8t", EighthTriplet, 3),
            ("16t", SixteenthTriplet, 6),
        ] {
            assert_eq!(GridDivision::parse(s), Some(d), "{s}");
            assert_eq!(d.label(), s);
            assert_eq!(serde_json::to_value(d).unwrap(), s);
            assert_eq!(d.steps_per_quarter(), spq, "{s}");
        }
        for bad in ["32", "", "16T", " 16", "2", "4t"] {
            assert_eq!(GridDivision::parse(bad), None, "{bad:?}");
        }
        assert_eq!(GridDivision::default(), Sixteenth);
    }
}
