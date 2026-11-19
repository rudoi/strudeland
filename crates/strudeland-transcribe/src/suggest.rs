//! Suggestions for the grid the notes were quantized to: a `--grid` division that fits the
//! rhythm clearly better, and a `--shift` that brings the notes onto the grid. Both only
//! suggest (a warning and a JSON field); the transcription keeps the requested grid.
//!
//! - Division: for every division that divides the bar, the rhythm fit ([`rhythm_fit`]: the gaps
//!   between onsets against whole steps, whatever the phase) is compared with the requested
//!   one's. When the requested division misfits (its fit at least `GRID_MISFIT`, worse than
//!   chance: the gaps fall between its steps, as dotted eighths do on eighths), the best other
//!   division is suggested if its fit is at most `GRID_SUGGEST_RATIO` of the requested one's and
//!   below chance (0.25), with its timing spread when quantized to it.
//! - Shift: on the suggested division when there is one (so the two suggestions go together),
//!   else on the requested one, each note's onset is a phase within its step; their
//!   confidence-weighted mean on the circle is the offset the notes share, and its length (0 for
//!   onsets at random, 1 when every note sits at the same place in its step) how consistent it
//!   is. With at least `SHIFT_MIN_NOTES` notes, a consistency of `SHIFT_MIN_CONSISTENCY` and an
//!   offset of `SHIFT_MIN_MS` or more, the notes are quantized again with the origin moved by it,
//!   and the shift (rounded to 5 ms) is suggested when the timing spread drops to at most
//!   `SHIFT_SUGGEST_RATIO` of the unshifted one on that division. The offset is within half a step either way: a
//!   placement a whole step or more away (a downbeat the singer felt elsewhere) is not seen.

use crate::consts::{
    GRID_FIT_MIN_PAIRS, GRID_MISFIT, GRID_SUGGEST_RATIO, SHIFT_MIN_CONSISTENCY, SHIFT_MIN_MS,
    SHIFT_MIN_NOTES, SHIFT_SUGGEST_RATIO,
};
use crate::grid::Grid;
use crate::quantize::{GridDivision, quantize, steps_per_bar};
use crate::segment::RawNote;

/// Every `--grid` division, coarse to fine.
const DIVISIONS: [GridDivision; 5] = [
    GridDivision::Quarter,
    GridDivision::Eighth,
    GridDivision::EighthTriplet,
    GridDivision::Sixteenth,
    GridDivision::SixteenthTriplet,
];

/// Chance for the rhythm fit: the mean distance of a uniform fraction from its nearest integer.
const CHANCE_FIT: f64 = 0.25;

/// A division that fits the rhythm better than the requested one.
#[derive(Debug, Clone, PartialEq)]
pub struct DivisionSuggestion {
    pub division: GridDivision,
    /// Its timing spread, milliseconds.
    pub spread_ms: f32,
}

/// The division to suggest instead of `requested`, if any.
pub fn suggest_division(
    raw: &[RawNote],
    grid: &Grid,
    requested: GridDivision,
) -> Option<DivisionSuggestion> {
    let own = rhythm_fit(raw, grid, requested)?;
    if own < GRID_MISFIT {
        return None;
    }
    let (division, fit) = DIVISIONS
        .iter()
        .filter(|&&d| d != requested)
        .filter_map(|&d| Some((d, rhythm_fit(raw, grid, d)?)))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    if !(fit <= GRID_SUGGEST_RATIO * own && fit < CHANCE_FIT) {
        return None;
    }
    let spread_ms = quantize(raw, grid, division).ok()?.timing_spread_ms;
    Some(DivisionSuggestion {
        division,
        spread_ms,
    })
}

/// How well the gaps between consecutive onsets of `raw` fit whole steps of `division`, whatever
/// the phase: the mean distance of each gap from its nearest whole number of steps, in steps
/// (0..=0.5; 0.25 for onsets at random), each gap weighted by the lower confidence of its two
/// notes. `None` under `GRID_FIT_MIN_PAIRS` gaps, for a division that does not divide the bar,
/// or without weight.
pub fn rhythm_fit(raw: &[RawNote], grid: &Grid, division: GridDivision) -> Option<f64> {
    steps_per_bar(grid.beats_per_bar, division).ok()?;
    let step = 60.0 / grid.bpm / f64::from(division.steps_per_quarter());
    if !(step.is_finite() && step > 0.0) {
        return None;
    }
    let mut notes: Vec<(f64, f64)> = raw
        .iter()
        .filter(|n| n.start.is_finite() && n.confidence.is_finite())
        .map(|n| (n.start, f64::from(n.confidence.clamp(0.0, 1.0))))
        .collect();
    notes.sort_by(|a, b| a.0.total_cmp(&b.0));
    if notes.len() < GRID_FIT_MIN_PAIRS + 1 {
        return None;
    }
    let (sum, total) = notes.windows(2).fold((0.0, 0.0), |(s, t), w| {
        let gap = (w[1].0 - w[0].0) / step;
        let weight = w[0].1.min(w[1].1);
        (s + weight * (gap - gap.round()).abs(), t + weight)
    });
    (total > 0.0).then(|| sum / total)
}

/// The shift in milliseconds (positive: the origin later) to suggest for `raw` on `grid` at
/// `division`, whose timing spread unshifted is `current_spread_ms`, if any.
pub fn suggest_shift(
    raw: &[RawNote],
    grid: &Grid,
    division: GridDivision,
    current_spread_ms: f32,
) -> Option<f64> {
    let step = 60.0 / grid.bpm / f64::from(division.steps_per_quarter());
    if !(step.is_finite() && step > 0.0) {
        return None;
    }
    // the confidence-weighted mean of each onset's phase within its step, on the circle
    let (mut x, mut y, mut total, mut count) = (0.0, 0.0, 0.0, 0usize);
    for n in raw
        .iter()
        .filter(|n| n.start.is_finite() && n.confidence.is_finite())
    {
        let w = f64::from(n.confidence.clamp(0.0, 1.0));
        let angle = std::f64::consts::TAU * (n.start - grid.origin_seconds) / step;
        x += w * angle.cos();
        y += w * angle.sin();
        total += w;
        count += 1;
    }
    if count < SHIFT_MIN_NOTES || total <= 0.0 {
        return None;
    }
    let consistency = x.hypot(y) / total;
    let shift_ms = y.atan2(x) / std::f64::consts::TAU * step * 1000.0;
    if consistency < SHIFT_MIN_CONSISTENCY || shift_ms.abs() < SHIFT_MIN_MS {
        return None;
    }
    let moved = Grid {
        origin_seconds: grid.origin_seconds + shift_ms / 1000.0,
        ..grid.clone()
    };
    let spread = quantize(raw, &moved, division).ok()?.timing_spread_ms;
    (f64::from(spread) <= SHIFT_SUGGEST_RATIO * f64::from(current_spread_ms))
        .then_some((shift_ms / 5.0).round() * 5.0)
}

/// The warning for a suggested division.
pub fn division_warning(s: &DivisionSuggestion, requested_spread_ms: f32) -> String {
    format!(
        "timing fits --grid {} better (spread {:.0} ms vs {:.0} ms)",
        s.division.label(),
        s.spread_ms,
        requested_spread_ms
    )
}

/// The warning for a suggested shift of `delta_ms` on top of the `--shift` already given,
/// measured on `suggested`, the suggested division, when there is one.
pub fn shift_warning(delta_ms: f64, given_ms: f64, suggested: Option<GridDivision>) -> String {
    let side = if delta_ms > 0.0 { "after" } else { "before" };
    let shift = given_ms + delta_ms;
    match suggested {
        Some(d) => format!(
            "notes sit ~{:.0} ms {side} the --grid {} grid; try --grid {} --shift {shift:.0}",
            delta_ms.abs(),
            d.label(),
            d.label()
        ),
        None => format!(
            "notes sit ~{:.0} ms {side} the grid; try --shift {shift:.0}",
            delta_ms.abs()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Resolved;
    use crate::report::Components;

    fn raw(start: f64, end: f64) -> RawNote {
        RawNote {
            start,
            end,
            midi: 60.0,
            components: Components {
                voiced: 1.0,
                stability: 1.0,
                fit: 1.0,
                onset: None,
            },
            confidence: 0.9,
            split_by_onset: false,
        }
    }

    /// 90 bpm in 12/8 (six quarter notes) from `origin`.
    fn grid(origin: f64) -> Grid {
        Grid {
            bpm: 90.0,
            beats_per_bar: 6.0,
            origin_seconds: origin,
            tempo_source: Resolved::Sidecar,
            phase_source: Resolved::Sidecar,
            confidence: None,
        }
    }

    /// Dotted eighths (0.5 s apart at 90 bpm) from `first`, with a small seeded jitter.
    fn dotted_eighths(first: f64, count: usize) -> Vec<RawNote> {
        (0..count)
            .map(|k| {
                let jitter = [0.012, -0.018, 0.005, -0.009, 0.015, -0.004][k % 6];
                let t = first + 0.5 * k as f64 + jitter;
                raw(t, t + 0.45)
            })
            .collect()
    }

    #[test]
    fn dotted_eighths_suggest_sixteenths_over_eighths() {
        let notes = dotted_eighths(2.0, 16);
        let g = grid(2.0);
        let eighths = quantize(&notes, &g, GridDivision::Eighth).unwrap();
        let s = suggest_division(&notes, &g, GridDivision::Eighth).expect("a suggestion");
        assert_eq!(s.division, GridDivision::Sixteenth);
        assert!(s.spread_ms < eighths.timing_spread_ms);
        assert_eq!(
            division_warning(&s, 150.4),
            format!(
                "timing fits --grid 16 better (spread {:.0} ms vs 150 ms)",
                s.spread_ms
            )
        );
        // asked for sixteenths, nothing fits better
        assert_eq!(suggest_division(&notes, &g, GridDivision::Sixteenth), None);
        // too few notes to judge
        let few = dotted_eighths(2.0, 6);
        assert_eq!(rhythm_fit(&few, &g, GridDivision::Eighth), None);
        assert_eq!(suggest_division(&few, &g, GridDivision::Eighth), None);
    }

    #[test]
    fn a_consistent_offset_suggests_a_shift() {
        // the notes sit 60 ms after the sixteenths (a sixteenth is 167 ms)
        let notes = dotted_eighths(2.06, 16);
        let g = grid(2.0);
        let q = quantize(&notes, &g, GridDivision::Sixteenth).unwrap();
        let shift = suggest_shift(&notes, &g, GridDivision::Sixteenth, q.timing_spread_ms)
            .expect("a shift");
        assert!((shift - 60.0).abs() <= 10.0, "{shift}");
        assert_eq!(
            shift_warning(60.0, 0.0, None),
            "notes sit ~60 ms after the grid; try --shift 60"
        );
        assert_eq!(
            shift_warning(-35.0, 300.0, None),
            "notes sit ~35 ms before the grid; try --shift 265"
        );
        assert_eq!(
            shift_warning(-35.0, 300.0, Some(GridDivision::Sixteenth)),
            "notes sit ~35 ms before the --grid 16 grid; try --grid 16 --shift 265"
        );
        // on the grid already: no shift
        let on = dotted_eighths(2.0, 16);
        let q = quantize(&on, &g, GridDivision::Sixteenth).unwrap();
        assert_eq!(
            suggest_shift(&on, &g, GridDivision::Sixteenth, q.timing_spread_ms),
            None
        );
        // onsets at random: no shift reduces the error clearly
        for seed in 1..=20 {
            let mut rng = crate::eval::synth::SplitMix64::new(seed);
            let mut t = 2.0;
            let random: Vec<RawNote> = (0..40)
                .map(|_| {
                    t += rng.uniform(0.2, 0.6);
                    raw(t, t + 0.15)
                })
                .collect();
            let q = quantize(&random, &g, GridDivision::Sixteenth).unwrap();
            assert_eq!(
                suggest_shift(&random, &g, GridDivision::Sixteenth, q.timing_spread_ms),
                None,
                "seed {seed}"
            );
            assert_eq!(
                suggest_division(&random, &g, GridDivision::Eighth),
                None,
                "seed {seed}"
            );
        }
    }
}
