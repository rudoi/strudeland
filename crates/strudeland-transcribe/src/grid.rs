//! The time grid: tempo, meter and bar 1's downbeat, from the hint or inferred from the onsets.
//!
//! Resolution, per part:
//!
//! - Tempo: the hint's `bpm` as given. Under `tempo_unheard` (a sidecar tempo the singer never
//!   heard) it is checked against [`infer_tempo`]: kept within `TEMPO_AGREE` modulo an octave (the
//!   inferred tempo is folded into `TEMPO_MIN..=TEMPO_MAX`, so 200 bpm is heard as 100), else
//!   replaced by the inferred tempo with a warning; with too few onsets to check, it is kept.
//!   Without a `bpm` it is inferred.
//! - Phase: the hint's `origin_seconds` as given, except a sidecar origin under `tempo_unheard`
//!   (the take followed no click, so its bar 1 is not trusted). Without one it is inferred by
//!   [`infer_phase`].
//! - Meter: the hint's `beats_per_bar`; without one 4.0, with the warning `meter assumed 4/4;
//!   pass --meter`.
//!
//! Hint values that are not finite (or, for `bpm` and `beats_per_bar`, not positive) count as
//! absent ([`GridHint::validated`], which `transcribe` applies). Inference needs at least
//! `MIN_ONSETS` onset peaks; with fewer, or when inference finds nothing, the grid is
//! [`GridOutcome::Unresolved`]. `shift_ms` moves the resolved origin, given
//! or inferred, later by that many milliseconds.
//!
//! Tempo inference ([`infer_tempo`]): the onset strength is smoothed by a Gaussian of
//! `TEMPO_SMOOTH_MS`, its mean removed, and autocorrelated. Each integer beat lag between
//! `TEMPO_MAX` and `TEMPO_MIN` scores its ACF plus `TEMPO_SUBDIV_WEIGHTS` times the ACF at a
//! quarter, a half and three quarters of the lag (a beat whose sixteenths also recur beats a lag
//! that only recurs itself, so a melody on a sixteenth grid does not lock onto three or six
//! sixteenths), weighted by a log-normal prior around `TEMPO_PRIOR_BPM` of `TEMPO_PRIOR_OCTAVES`
//! (librosa's tempo prior, narrowed). The best lag is refined by parabolic interpolation, then
//! again at the ACF peaks near `TEMPO_REFINE_BEATS` times the lag (a peak many beats out pins the
//! period finely), and the tempo octave-folded into `TEMPO_MIN..=TEMPO_MAX`. Its confidence is the
//! ACF at the lag over the ACF at lag 0.
//!
//! Phase inference ([`infer_phase`]): candidate phases every `PHASE_STEP_MS` over one bar each
//! score the onset peaks near a beat, plus `PHASE_DOWNBEAT_WEIGHT` times those near a downbeat;
//! a peak counts its strength times `1 - distance / PHASE_WINDOW_MS`. The downbeat term only
//! settles which offset within the beat the grid has (the beats alone leave a melody on a
//! sixteenth grid ambiguous). Ties go to the candidate whose earliest strong onset (at least
//! `STRONG_ONSET` of the largest peak) on its beat grid comes first. The best phase is refined by
//! the weighted mean deviation of the peaks near its beats. Bar 1 is the beat of the earliest
//! strong onset on that grid (the earliest onset on it when none is strong): alignment is not
//! trusted to find the downbeat. The confidence is `x = best * candidates / sum of scores` (1.0
//! is chance) mapped to 0..1 by `(x - 1) / x`.

use serde::{Deserialize, Serialize};

use crate::consts::{
    MIN_ONSETS, PHASE_DOWNBEAT_WEIGHT, PHASE_MAX_CANDIDATES, PHASE_STEP_MS, PHASE_WINDOW_MS,
    STRONG_ONSET, TEMPO_AGREE, TEMPO_MAX, TEMPO_MIN, TEMPO_PRIOR_BPM, TEMPO_PRIOR_OCTAVES,
    TEMPO_REFINE_BEATS, TEMPO_SMOOTH_MS, TEMPO_SUBDIV_WEIGHTS,
};
use crate::error::TranscribeError;
use crate::onset::OnsetTrack;

/// What the caller knows about the grid, flags and sidecar already merged per field.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GridHint {
    /// Quarter notes per minute.
    pub bpm: Option<Hinted<f64>>,
    /// Quarter notes per bar.
    pub beats_per_bar: Option<Hinted<f64>>,
    /// Bar 1's downbeat, memo seconds.
    pub origin_seconds: Option<Hinted<f64>>,
    /// The meter as written (`--meter`, or the sidecar's `tempo.meter`), reported as the grid's
    /// `meter` with `beats_per_bar`; without it the report derives one from the beats per bar.
    pub meter: Option<String>,
    /// A sidecar tempo the take never heard (no song and no metronome).
    pub tempo_unheard: bool,
}

/// A hint value and where it came from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hinted<T> {
    pub value: T,
    pub source: Resolved,
}

impl<T> Hinted<T> {
    /// A value from a command-line flag.
    pub fn flag(value: T) -> Self {
        Self {
            value,
            source: Resolved::Flag,
        }
    }

    /// A value from the memo's sidecar.
    pub fn sidecar(value: T) -> Self {
        Self {
            value,
            source: Resolved::Sidecar,
        }
    }
}

impl GridHint {
    /// Memo seconds from bar `from`'s downbeat to the end of bar `to` (bars count from 1), on
    /// the grid [`resolve_grid`] gives this hint: bar 1 moved later by `shift_ms`, as the grid's
    /// origin is. `None` without a usable tempo, meter and bar 1, and under an unheard sidecar
    /// tempo ([`GridHint::sidecar_unheard`]): that tempo is checked against the audio and its
    /// sidecar bar 1 ignored, so the grid is not known before the audio is read.
    pub fn bar_span(&self, from: u32, to: u32, shift_ms: f64) -> Option<(f64, f64)> {
        let hint = self.validated();
        if hint.sidecar_unheard() {
            return None;
        }
        let (bpm, beats, origin) = (
            hint.bpm?.value,
            hint.beats_per_bar?.value,
            hint.origin_seconds?.value + shift_ms / 1000.0,
        );
        if !origin.is_finite() {
            return None;
        }
        let bar = beats * 60.0 / bpm;
        Some((
            origin + (f64::from(from) - 1.0) * bar,
            origin + f64::from(to) * bar,
        ))
    }

    /// Whether the tempo is the sidecar's and the take never heard it: [`resolve_grid`] then
    /// checks the tempo against the audio and ignores a sidecar bar 1.
    pub(crate) fn sidecar_unheard(&self) -> bool {
        self.tempo_unheard && self.bpm.is_some_and(|h| h.source == Resolved::Sidecar)
    }

    /// The hint with its unusable values removed: a value that is not finite (or, for `bpm` and
    /// `beats_per_bar`, not positive) counts as absent, and so does `meter` without a
    /// `beats_per_bar`.
    pub(crate) fn validated(&self) -> Self {
        let positive = |v: Option<Hinted<f64>>| v.filter(|h| h.value.is_finite() && h.value > 0.0);
        let beats_per_bar = positive(self.beats_per_bar);
        Self {
            bpm: positive(self.bpm),
            beats_per_bar,
            origin_seconds: self.origin_seconds.filter(|h| h.value.is_finite()),
            meter: self.meter.clone().filter(|_| beats_per_bar.is_some()),
            tempo_unheard: self.tempo_unheard,
        }
    }
}

/// Where a resolved grid part came from; serialized as `"flag"`, `"sidecar"` or `"inferred"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolved {
    Flag,
    Sidecar,
    Inferred,
}

impl Resolved {
    /// The report's name: `"flag"`, `"sidecar"` or `"inferred"`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Sidecar => "sidecar",
            Self::Inferred => "inferred",
        }
    }
}

/// The resolved grid.
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// Quarter notes per minute.
    pub bpm: f64,
    /// Quarter notes per bar.
    pub beats_per_bar: f64,
    /// Bar 1's downbeat, memo seconds (the shift applied).
    pub origin_seconds: f64,
    pub tempo_source: Resolved,
    pub phase_source: Resolved,
    /// The minimum of the inferred parts' confidences; `None` when nothing was inferred.
    pub confidence: Option<f32>,
}

/// The grid with the warnings its resolution raised, or no grid.
#[derive(Debug, Clone, PartialEq)]
pub enum GridOutcome {
    Known(Grid, Vec<String>),
    Unresolved(Vec<String>),
}

/// The warning when inference has too little to go on.
pub const UNRESOLVED_WARNING: &str =
    "too few onsets to infer a grid; notes are not quantized. Pass --bpm (and --bar1)";

/// The warning when the meter is assumed.
pub const METER_ASSUMED_WARNING: &str = "meter assumed 4/4; pass --meter";

/// Resolves the grid from `hint`, inferring what it lacks from `onsets` (times from the start of
/// the analysed span, which starts `span_start` seconds into the memo). `memo_seconds` is the
/// memo's length: a hint origin the grid would use (not an ignored unheard sidecar one) past it is
/// `Bar1PastEnd`. `shift_ms` moves the origin later.
/// `transcribe` passes a [`GridHint::validated`] hint and finite numbers.
pub fn resolve_grid(
    hint: &GridHint,
    onsets: &OnsetTrack,
    span_start: f64,
    memo_seconds: f64,
    shift_ms: f64,
) -> Result<GridOutcome, TranscribeError> {
    let unheard = hint.sidecar_unheard();
    // an unheard sidecar's bar 1 is ignored: the take followed no click
    let given_origin = hint
        .origin_seconds
        .filter(|h| !(unheard && h.source == Resolved::Sidecar));
    if let Some(Hinted {
        value: bar1,
        source,
    }) = given_origin
        && bar1 > memo_seconds
    {
        return Err(TranscribeError::Bar1PastEnd {
            bar1,
            len: memo_seconds,
            from: source,
        });
    }

    let unresolved = || {
        Ok(GridOutcome::Unresolved(vec![
            UNRESOLVED_WARNING.to_string(),
        ]))
    };
    let enough = onsets.peaks.len() >= MIN_ONSETS;
    let beats_per_bar = hint.beats_per_bar.map_or(4.0, |h| h.value);
    let mut warnings = Vec::new();

    let (bpm, tempo_source, tempo_conf) = match hint.bpm {
        Some(Hinted { value, source }) if !unheard => (value, source, None),
        // too few onsets to check it: the sidecar tempo is kept (the phase still needs a flag)
        Some(Hinted { value: bpm, .. }) if !enough => (bpm, Resolved::Sidecar, None),
        Some(Hinted { value: bpm, .. }) => match infer_tempo(onsets) {
            Some((heard, conf)) if !tempo_agrees(heard, bpm) => {
                warnings.push(format!(
                    "the take did not follow the song's tempo ({} bpm, heard {} bpm)",
                    trim_number(bpm),
                    trim_number(heard)
                ));
                (heard, Resolved::Inferred, Some(conf))
            }
            _ => (bpm, Resolved::Sidecar, None),
        },
        None => {
            if !enough {
                return unresolved();
            }
            match infer_tempo(onsets) {
                Some((bpm, conf)) => (bpm, Resolved::Inferred, Some(conf)),
                None => return unresolved(),
            }
        }
    };

    let (origin, phase_source, phase_conf) = match given_origin {
        Some(Hinted { value, source }) => (value, source, None),
        None => {
            if !enough {
                return unresolved();
            }
            match infer_phase(onsets, bpm, beats_per_bar) {
                Some((at, conf)) => (span_start + at, Resolved::Inferred, Some(conf)),
                None => return unresolved(),
            }
        }
    };

    let confidence = [tempo_conf, phase_conf]
        .into_iter()
        .flatten()
        .reduce(f32::min);
    let shown = confidence.unwrap_or(0.0);
    match (tempo_source, phase_source) {
        (Resolved::Inferred, Resolved::Inferred) => warnings.push(format!(
            "tempo and phase inferred from the audio ({bpm:.1} bpm, confidence {shown:.2}); pass --bpm if you know it"
        )),
        (Resolved::Inferred, _) => warnings.push(format!(
            "tempo inferred from the audio ({bpm:.1} bpm, confidence {shown:.2}); pass --bpm if you know it"
        )),
        (_, Resolved::Inferred) => warnings.push(format!(
            "phase inferred from the audio (confidence {shown:.2}); beat 1 is the first strong onset, so check it with the user, or pass --bar1"
        )),
        _ => {}
    }
    if hint.beats_per_bar.is_none() {
        warnings.push(METER_ASSUMED_WARNING.to_string());
    }

    Ok(GridOutcome::Known(
        Grid {
            bpm,
            beats_per_bar,
            origin_seconds: origin + shift_ms / 1000.0,
            tempo_source,
            phase_source,
            confidence,
        },
        warnings,
    ))
}

/// The tempo of the onset strength in bpm, folded into `TEMPO_MIN..=TEMPO_MAX`, and its
/// confidence; `None` with fewer than `MIN_ONSETS` peaks or no periodicity.
pub fn infer_tempo(onsets: &OnsetTrack) -> Option<(f64, f32)> {
    let hop = f64::from(onsets.hop_seconds);
    if onsets.peaks.len() < MIN_ONSETS || !(hop.is_finite() && hop > 0.0) {
        return None;
    }
    let lag_lo = 60.0 / TEMPO_MAX / hop;
    let lag_hi = 60.0 / TEMPO_MIN / hop;
    let max_lag = lag_hi.ceil() as usize + 1;

    let signal = smoothed_centred(&onsets.strength, f64::from(TEMPO_SMOOTH_MS) / 1000.0 / hop);
    let acf: Vec<f64> = (0..=max_lag).map(|lag| lag_product(&signal, lag)).collect();
    let zero = *acf.first()?;
    if zero <= 0.0 {
        return None;
    }
    let at = |lag: f64| interpolate(&acf, lag);
    let score = |lag: f64| {
        let pulse = at(lag)
            + TEMPO_SUBDIV_WEIGHTS
                .iter()
                .map(|&(fraction, weight)| weight * at(lag * fraction))
                .sum::<f64>();
        pulse * prior(60.0 / (lag * hop))
    };

    let first = (lag_lo.floor() as usize).max(2);
    let last = (lag_hi.ceil() as usize).max(first);
    let mut best: Option<(usize, f64)> = None;
    for lag in first..=last {
        let s = score(lag as f64);
        if best.is_none_or(|(_, b)| s > b) {
            best = Some((lag, s));
        }
    }
    let (lag, peak) = best?;
    if peak <= 0.0 {
        return None;
    }
    let lag = lag as f64 + parabolic_offset(score(lag as f64 - 1.0), peak, score(lag as f64 + 1.0));
    let confidence = (at(lag) / zero).clamp(0.0, 1.0) as f32;
    let lag = refine_lag(&signal, lag);
    Some((fold_tempo(60.0 / (lag * hop)), confidence))
}

/// The beat lag refined at its multiples `TEMPO_REFINE_BEATS`: the ACF's maximum within an
/// eighth of a beat of `k * lag`, parabolically interpolated and divided by `k`. Multiples past
/// half the signal are skipped.
fn refine_lag(signal: &[f64], lag: f64) -> f64 {
    let mut lag = lag;
    for k in TEMPO_REFINE_BEATS {
        let k = f64::from(k);
        let target = k * lag;
        let reach = (lag / 8.0).max(1.0);
        if target + reach + 1.0 > signal.len() as f64 / 2.0 {
            break;
        }
        let lo = (target - reach).floor().max(1.0) as usize;
        let hi = (target + reach).ceil() as usize;
        let mut best: Option<(usize, f64)> = None;
        for l in lo..=hi {
            let v = lag_product(signal, l);
            if best.is_none_or(|(_, b)| v > b) {
                best = Some((l, v));
            }
        }
        let Some((l, v)) = best else { break };
        if v <= 0.0 || l == lo || l == hi {
            // no peak inside the window: keep the coarser lag
            break;
        }
        let offset = parabolic_offset(lag_product(signal, l - 1), v, lag_product(signal, l + 1));
        lag = (l as f64 + offset) / k;
    }
    lag
}

/// The vertex offset, within -0.5..=0.5, of the parabola through three equally spaced values
/// whose middle one is the largest; 0 when they are not a peak.
fn parabolic_offset(left: f64, mid: f64, right: f64) -> f64 {
    let denom = left - 2.0 * mid + right;
    if denom < 0.0 {
        (0.5 * (left - right) / denom).clamp(-0.5, 0.5)
    } else {
        0.0
    }
}

/// Bar 1's downbeat in seconds from the start of the analysed span (the beat of the earliest
/// aligned strong onset) and the phase confidence; `None` with fewer than `MIN_ONSETS` peaks, a
/// tempo or meter that is not a positive finite number, or no peak near any candidate grid.
pub fn infer_phase(onsets: &OnsetTrack, bpm: f64, beats_per_bar: f64) -> Option<(f64, f32)> {
    if onsets.peaks.len() < MIN_ONSETS
        || !(bpm.is_finite() && bpm > 0.0)
        || !(beats_per_bar.is_finite() && beats_per_bar > 0.0)
    {
        return None;
    }
    let peaks: Vec<(f64, f64)> = onsets
        .peaks
        .iter()
        .filter(|p| p.seconds.is_finite() && p.strength.is_finite() && p.strength > 0.0)
        .map(|p| (p.seconds, f64::from(p.strength)))
        .collect();
    let max_strength = peaks.iter().fold(0.0f64, |m, p| m.max(p.1));
    if max_strength <= 0.0 {
        return None;
    }
    let strong = f64::from(STRONG_ONSET) * max_strength;
    let beat = 60.0 / bpm;
    let bar = beat * beats_per_bar;
    let window = (f64::from(PHASE_WINDOW_MS) / 1000.0).min(beat / 2.0);
    let step = (f64::from(PHASE_STEP_MS) / 1000.0).max(bar / PHASE_MAX_CANDIDATES as f64);
    let count = ((bar / step).ceil() as usize).clamp(1, PHASE_MAX_CANDIDATES);

    // signed deviation of `t` from the nearest point of a grid of `period` through `phase`
    let deviation = |t: f64, phase: f64, period: f64| {
        let x = (t - phase).rem_euclid(period);
        if x > period / 2.0 { x - period } else { x }
    };
    let weight = |d: f64| (1.0 - d.abs() / window).max(0.0);
    let score = |phase: f64| {
        peaks
            .iter()
            .map(|&(t, s)| {
                s * (weight(deviation(t, phase, beat))
                    + PHASE_DOWNBEAT_WEIGHT * weight(deviation(t, phase, bar)))
            })
            .sum::<f64>()
    };
    // the earliest strong onset on the beat grid of a phase
    let first_strong = |phase: f64| {
        peaks
            .iter()
            .filter(|&&(t, s)| s >= strong && deviation(t, phase, beat).abs() < window)
            .map(|p| p.0)
            .reduce(f64::min)
    };

    let scores: Vec<f64> = (0..count).map(|j| score(j as f64 * step)).collect();
    let best = scores.iter().copied().fold(0.0f64, f64::max);
    let total: f64 = scores.iter().sum();
    if best <= 0.0 || total <= 0.0 {
        return None;
    }
    let tolerance = best * 1e-9;
    let mut chosen: Option<(f64, f64)> = None; // (phase, earliest strong onset)
    for (j, &s) in scores.iter().enumerate() {
        if s < best - tolerance {
            continue;
        }
        let phase = j as f64 * step;
        let earliest = first_strong(phase).unwrap_or(f64::INFINITY);
        if chosen.is_none_or(|(_, e)| earliest < e) {
            chosen = Some((phase, earliest));
        }
    }
    let (phase, _) = chosen?;

    // refine by the weighted mean deviation of the peaks aligned with the beats
    let (sum_w, sum_wd) = peaks
        .iter()
        .map(|&(t, s)| {
            let d = deviation(t, phase, beat);
            let w = s * weight(d);
            (w, w * d)
        })
        .fold((0.0, 0.0), |(a, b), (w, wd)| (a + w, b + wd));
    let phase = if sum_w > 0.0 {
        phase + sum_wd / sum_w
    } else {
        phase
    };

    // bar 1 is the beat of the earliest aligned strong onset, else of the earliest aligned one
    let aligned = |min: f64| {
        peaks
            .iter()
            .filter(|&&(t, s)| s >= min && deviation(t, phase, beat).abs() < window)
            .map(|&(t, _)| t)
            .reduce(f64::min)
    };
    let anchor = aligned(strong).or_else(|| aligned(0.0))?;
    let origin = anchor - deviation(anchor, phase, beat);

    let x = best * count as f64 / total;
    let confidence = ((x - 1.0) / x).clamp(0.0, 1.0) as f32;
    Some((origin, confidence))
}

/// Whether the `heard` tempo (octave-folded by [`infer_tempo`]) agrees with `bpm` modulo an
/// octave: within `TEMPO_AGREE` of `bpm` times the power of two nearest their ratio, so a 200 bpm
/// song heard as 100 bpm agrees.
fn tempo_agrees(heard: f64, bpm: f64) -> bool {
    let octave = (heard / bpm).log2().round();
    let target = bpm * octave.exp2();
    target.is_finite() && target > 0.0 && (heard - target).abs() / target <= TEMPO_AGREE
}

/// `bpm` halved or doubled into `TEMPO_MIN..=TEMPO_MAX`; a value that is not a positive finite
/// number is returned unchanged.
pub fn fold_tempo(bpm: f64) -> f64 {
    if !(bpm.is_finite() && bpm > 0.0) {
        return bpm;
    }
    let mut bpm = bpm;
    while bpm > TEMPO_MAX {
        bpm /= 2.0;
    }
    while bpm < TEMPO_MIN {
        bpm *= 2.0;
    }
    bpm
}

/// `"{beatsPerBar}/4"` for a whole number of quarter notes, else the smallest of `/8` and `/16`
/// that makes the numerator whole (`1.5` is `"3/8"`, `1.75` is `"7/16"`).
pub fn meter_label(beats_per_bar: f64) -> String {
    for (scale, denominator) in [(1.0, 4), (2.0, 8), (4.0, 16)] {
        let n = beats_per_bar * scale;
        if n.is_finite() && (n - n.round()).abs() < 1e-9 {
            return format!("{}/{denominator}", n.round());
        }
    }
    format!("{}/8", trim_number(beats_per_bar * 2.0))
}

/// `--meter N/D` as quarter notes per bar, `N * 4 / D`, with `N >= 1` and `D` one of 1, 2, 4, 8
/// and 16.
pub fn parse_meter(s: &str) -> Result<f64, TranscribeError> {
    let bad =
        || TranscribeError::BadOption(format!("bad --meter \"{s}\"; use N/D such as 4/4 or 6/8"));
    let (n, d) = s.split_once('/').ok_or_else(bad)?;
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit());
    if !digits(n) || !digits(d) {
        return Err(bad());
    }
    let n: u32 = n.parse().map_err(|_| bad())?;
    let d: u32 = d.parse().map_err(|_| bad())?;
    if n == 0 || ![1, 2, 4, 8, 16].contains(&d) {
        return Err(bad());
    }
    Ok(f64::from(n) * 4.0 / f64::from(d))
}

/// A number with one decimal, a trailing `.0` dropped (`120`, `98.4`).
fn trim_number(x: f64) -> String {
    let s = format!("{x:.1}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_string)
}

/// The log-normal tempo prior.
fn prior(bpm: f64) -> f64 {
    let octaves = (bpm / TEMPO_PRIOR_BPM).log2() / TEMPO_PRIOR_OCTAVES;
    (-0.5 * octaves * octaves).exp()
}

/// `strength` (non-finite values as 0) smoothed by a Gaussian of `sigma` frames, mean removed.
fn smoothed_centred(strength: &[f32], sigma: f64) -> Vec<f64> {
    let values: Vec<f64> = strength
        .iter()
        .map(|&v| if v.is_finite() { f64::from(v) } else { 0.0 })
        .collect();
    let smoothed = if sigma.is_finite() && sigma > 0.0 {
        let half = (3.0 * sigma).ceil() as usize;
        let kernel: Vec<f64> = (0..=2 * half)
            .map(|k| {
                let x = (k as f64 - half as f64) / sigma;
                (-0.5 * x * x).exp()
            })
            .collect();
        let norm: f64 = kernel.iter().sum();
        (0..values.len())
            .map(|i| {
                kernel
                    .iter()
                    .enumerate()
                    .filter_map(|(k, w)| {
                        (i + k)
                            .checked_sub(half)
                            .and_then(|j| values.get(j))
                            .map(|v| v * w)
                    })
                    .sum::<f64>()
                    / norm
            })
            .collect()
    } else {
        values
    };
    let mean = if smoothed.is_empty() {
        0.0
    } else {
        smoothed.iter().sum::<f64>() / smoothed.len() as f64
    };
    smoothed.iter().map(|v| v - mean).collect()
}

/// `sum_i x[i] * x[i + lag]`: the autocorrelation at one lag (0 past the signal's length).
fn lag_product(x: &[f64], lag: usize) -> f64 {
    x.iter().zip(x.iter().skip(lag)).map(|(a, b)| a * b).sum()
}

/// `values` at a fractional index by linear interpolation; 0 outside.
fn interpolate(values: &[f64], at: f64) -> f64 {
    if !(at.is_finite() && at >= 0.0) {
        return 0.0;
    }
    let i = at.floor() as usize;
    let frac = at - i as f64;
    let a = values.get(i).copied().unwrap_or(0.0);
    let b = values.get(i + 1).copied().unwrap_or(0.0);
    a + frac * (b - a)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::synth::{Case, SynthSettings};
    use crate::onset::Onset;
    use crate::test_support::hop;

    /// An onset track whose strength is an impulse of each peak's strength at its frame, `seconds`
    /// long.
    fn onsets_at(peaks: &[(f64, f32)], seconds: f64) -> OnsetTrack {
        let hop_seconds = hop();
        let n = (seconds / f64::from(hop_seconds)).ceil() as usize + 1;
        let mut strength = vec![0.0f32; n];
        for &(t, s) in peaks {
            let i = (t / f64::from(hop_seconds)).round() as usize;
            if let Some(v) = strength.get_mut(i) {
                *v = v.max(s);
            }
        }
        OnsetTrack {
            hop_seconds,
            strength,
            rms: Vec::new(),
            peaks: peaks
                .iter()
                .map(|&(seconds, strength)| Onset { seconds, strength })
                .collect(),
        }
    }

    /// A melody on a sixteenth grid at `bpm`: a fixed rhythm of onsets (in sixteenths within each
    /// bar, the downbeat strongest) repeated over `bars` 4/4 bars from `origin`.
    fn melody(bpm: f64, origin: f64, bars: u32) -> OnsetTrack {
        // two bars of rhythm, alternating
        let rhythms: [&[u32]; 4] = [
            &[0, 3, 4, 6, 8, 11, 12],
            &[0, 2, 5, 8, 10, 14],
            &[0, 4, 7, 8, 12, 13],
            &[0, 1, 4, 6, 9, 12],
        ];
        let sixteenth = 60.0 / bpm / 4.0;
        let mut peaks = Vec::new();
        for bar in 0..bars {
            for &step in rhythms[bar as usize % rhythms.len()] {
                let t = origin + f64::from(bar * 16 + step) * sixteenth;
                let s = if step == 0 {
                    1.0
                } else {
                    0.6 + 0.05 * (step % 5) as f32
                };
                peaks.push((t, s));
            }
        }
        let end = origin + f64::from(bars) * 16.0 * sixteenth + 0.5;
        onsets_at(&peaks, end)
    }

    /// The rhythm of the synthetic generator's melody for `seed` (random cuts of each 4/4 bar on
    /// a sixteenth grid, downbeats strongest) at `bpm` from `origin`, as exact onset peaks.
    fn generated_rhythm(seed: u64, bpm: f64, origin: f64) -> OnsetTrack {
        let case = Case::generate(seed, &SynthSettings::default());
        let sixteenth = 60.0 / bpm / 4.0;
        let peaks: Vec<(f64, f32)> = case
            .truth
            .iter()
            .map(|n| {
                let at = origin + f64::from((n.bar - 1) * 16 + n.step) * sixteenth;
                (at, if n.step == 0 { 1.0 } else { 0.7 })
            })
            .collect();
        onsets_at(&peaks, origin + f64::from(case.bars * 16) * sixteenth + 0.5)
    }

    fn sidecar_hint() -> GridHint {
        GridHint {
            bpm: Some(Hinted::sidecar(100.0)),
            beats_per_bar: Some(Hinted::sidecar(4.0)),
            origin_seconds: Some(Hinted::sidecar(2.4)),
            ..GridHint::default()
        }
    }

    fn known(outcome: GridOutcome) -> (Grid, Vec<String>) {
        match outcome {
            GridOutcome::Known(grid, warnings) => (grid, warnings),
            GridOutcome::Unresolved(w) => panic!("expected a grid, got unresolved: {w:?}"),
        }
    }

    fn empty_onsets() -> OnsetTrack {
        onsets_at(&[], 1.0)
    }

    #[test]
    fn sidecar_grid_is_used() {
        let outcome = resolve_grid(&sidecar_hint(), &empty_onsets(), 5.0, 30.0, 0.0).unwrap();
        let (grid, warnings) = known(outcome);
        assert_eq!(
            grid,
            Grid {
                bpm: 100.0,
                beats_per_bar: 4.0,
                origin_seconds: 2.4,
                tempo_source: Resolved::Sidecar,
                phase_source: Resolved::Sidecar,
                confidence: None,
            }
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn flags_override_sidecar_per_field() {
        let hint = GridHint {
            bpm: Some(Hinted::flag(90.0)),
            beats_per_bar: Some(Hinted::sidecar(3.0)),
            origin_seconds: Some(Hinted::flag(1.25)),
            meter: None,
            tempo_unheard: true, // a flag tempo is never second-guessed
        };
        let (grid, warnings) = known(resolve_grid(&hint, &empty_onsets(), 0.0, 30.0, 0.0).unwrap());
        assert_eq!(grid.bpm, 90.0);
        assert_eq!(grid.beats_per_bar, 3.0);
        assert_eq!(grid.origin_seconds, 1.25);
        assert_eq!(grid.tempo_source, Resolved::Flag);
        assert_eq!(grid.phase_source, Resolved::Flag);
        assert_eq!(grid.confidence, None);
        assert!(warnings.is_empty(), "{warnings:?}");

        let hint = GridHint {
            bpm: Some(Hinted::sidecar(90.0)),
            origin_seconds: Some(Hinted::sidecar(1.25)),
            tempo_unheard: false,
            ..hint
        };
        let (grid, _) = known(resolve_grid(&hint, &empty_onsets(), 0.0, 30.0, 0.0).unwrap());
        assert_eq!(grid.tempo_source, Resolved::Sidecar);
        assert_eq!(grid.phase_source, Resolved::Sidecar);
        for (source, name) in [
            (Resolved::Flag, "flag"),
            (Resolved::Sidecar, "sidecar"),
            (Resolved::Inferred, "inferred"),
        ] {
            assert_eq!(source.label(), name);
            assert_eq!(serde_json::to_value(source).unwrap(), name);
        }
    }

    #[test]
    fn phase_inferred_from_onsets() {
        for (bpm, origin) in [(120.0, 1.337), (97.0, 0.61), (150.0, 2.02)] {
            let mut track = melody(bpm, origin, 4);
            // a weak pickup a sixteenth before bar 1
            let pickup = origin - 60.0 / bpm / 4.0;
            track.peaks.insert(
                0,
                Onset {
                    seconds: pickup,
                    strength: 0.3,
                },
            );
            let hint = GridHint {
                bpm: Some(Hinted::sidecar(bpm)),
                beats_per_bar: Some(Hinted::sidecar(4.0)),
                ..GridHint::default()
            };
            let (grid, warnings) = known(resolve_grid(&hint, &track, 10.0, 60.0, 0.0).unwrap());
            let err = grid.origin_seconds - (10.0 + origin);
            assert!(err.abs() <= 0.025, "{bpm} bpm: origin off by {err} s");
            assert_eq!(grid.tempo_source, Resolved::Sidecar);
            assert_eq!(grid.phase_source, Resolved::Inferred);
            let conf = grid.confidence.expect("a phase confidence");
            assert!(
                (0.0..=1.0).contains(&conf) && conf > 0.5,
                "{bpm} bpm: confidence {conf}"
            );
            assert_eq!(
                warnings,
                vec![format!(
                    "phase inferred from the audio (confidence {conf:.2}); beat 1 is the first strong onset, so check it with the user, or pass --bar1"
                )]
            );
        }
    }

    #[test]
    fn phase_recovers_late_onsets_exactly() {
        // every onset 10 ms late: the refined phase follows them
        let track = melody(120.0, 1.01, 4);
        let (origin, _) = infer_phase(&track, 120.0, 4.0).unwrap();
        assert!((origin - 1.01).abs() < 1e-6, "origin {origin}");
    }

    #[test]
    fn phase_ties_go_to_earliest_strong_onset() {
        // equal onsets every half beat at 120 bpm tie both half-beat phases; weak onsets come
        // first on both, the first strong one is at 0.75 s
        let mut peaks = vec![(0.25, 0.2), (0.5, 0.2)];
        for k in 0..8 {
            peaks.push((0.75 + 0.25 * f64::from(k), 1.0));
        }
        // the two half-beat phases tie on the beats (a weak onset and four strong ones each)
        // and on the downbeats (a weak onset and a strong one each)
        let track = onsets_at(&peaks, 3.0);
        let (origin, conf) = infer_phase(&track, 120.0, 4.0).unwrap();
        assert!((origin - 0.75).abs() < 1e-9, "origin {origin}");
        assert!((0.0..=1.0).contains(&conf));
    }

    #[test]
    fn tempo_inferred_within_2_percent() {
        for bpm in [90.0, 120.0, 150.0] {
            let track = generated_rhythm(1, bpm, 0.73);
            let (got, conf) = infer_tempo(&track).expect("a tempo");
            let err = (got - bpm).abs() / bpm;
            assert!(
                err <= 0.02,
                "{bpm} bpm: inferred {got} ({:.2}%)",
                err * 100.0
            );
            assert!((0.0..=1.0).contains(&conf), "{bpm} bpm: confidence {conf}");

            let hint = GridHint::default();
            let (grid, warnings) = known(resolve_grid(&hint, &track, 0.0, 60.0, 0.0).unwrap());
            assert_eq!(grid.tempo_source, Resolved::Inferred);
            assert_eq!(grid.phase_source, Resolved::Inferred);
            assert!(
                (grid.origin_seconds - 0.73).abs() <= 0.025,
                "{bpm}: {grid:?}"
            );
            let shown = grid.confidence.expect("confidence");
            assert_eq!(
                warnings,
                vec![
                    format!(
                        "tempo and phase inferred from the audio ({:.1} bpm, confidence {shown:.2}); pass --bpm if you know it",
                        grid.bpm
                    ),
                    METER_ASSUMED_WARNING.to_string(),
                ],
                "{bpm} bpm"
            );
            assert_eq!(grid.beats_per_bar, 4.0);
        }
    }

    /// The first `bars` bars of the generator's clean case `seed`.
    fn short_case(seed: u64, bars: u32) -> Case {
        let mut case = Case::generate(seed, &SynthSettings::default());
        case.truth.retain(|n| n.bar <= bars);
        case.bars = bars;
        case
    }

    #[test]
    fn tempo_and_phase_inferred_from_synth_audio() {
        for seed in [1000, 1003] {
            let case = short_case(seed, 6);
            let onsets = crate::onset::onset_strength(&case.render(crate::consts::ANALYSIS_RATE));
            let (grid, _) =
                known(resolve_grid(&GridHint::default(), &onsets, 0.0, 60.0, 0.0).unwrap());
            let err = (grid.bpm - case.bpm) / case.bpm;
            assert!(
                err.abs() <= 0.02,
                "seed {seed}: {} bpm for {}",
                grid.bpm,
                case.bpm
            );
            // onsets are a little late (the synth's attack), never more than the phase window
            let off = grid.origin_seconds - case.origin_seconds;
            assert!(
                (0.0..=0.025).contains(&off),
                "seed {seed}: origin {off:+.3} s off"
            );
        }
    }

    #[test]
    fn tempo_inferred_from_a_beat_train() {
        for bpm in [90.0, 120.0] {
            let beat = 60.0 / bpm;
            let peaks: Vec<(f64, f32)> =
                (0..16).map(|k| (0.5 + beat * f64::from(k), 1.0)).collect();
            let (got, _) = infer_tempo(&onsets_at(&peaks, 0.5 + 16.0 * beat)).unwrap();
            assert!((got - bpm).abs() / bpm <= 0.02, "{bpm} bpm: inferred {got}");
        }
    }

    #[test]
    fn tempo_octave_folded_into_range() {
        assert_eq!(fold_tempo(240.0), 120.0);
        assert_eq!(fold_tempo(400.0), 100.0);
        assert_eq!(fold_tempo(45.0), 90.0);
        assert_eq!(fold_tempo(25.0), 100.0);
        assert_eq!(fold_tempo(180.0), 180.0);
        assert_eq!(fold_tempo(60.0), 60.0);
        assert!(fold_tempo(f64::NAN).is_nan());
        assert_eq!(fold_tempo(0.0), 0.0);
        // a pulse faster than the range lands inside it
        let peaks: Vec<(f64, f32)> = (0..40).map(|k| (0.3 + 0.15 * f64::from(k), 1.0)).collect();
        let (got, _) = infer_tempo(&onsets_at(&peaks, 7.0)).unwrap();
        assert!((TEMPO_MIN..=TEMPO_MAX).contains(&got), "inferred {got}");
        assert!(
            (got - 100.0).abs() / 100.0 <= 0.02,
            "400 bpm folds to 100: inferred {got}"
        );
    }

    fn unheard_hint(bpm: f64) -> GridHint {
        GridHint {
            bpm: Some(Hinted::sidecar(bpm)),
            beats_per_bar: Some(Hinted::sidecar(4.0)),
            origin_seconds: Some(Hinted::sidecar(9.0)), // ignored: the take heard no click
            tempo_unheard: true,
            ..GridHint::default()
        }
    }

    #[test]
    fn unheard_tempo_kept_within_4_percent() {
        let track = melody(120.0, 0.5, 8);
        for sidecar in [116.0, 120.0, 124.5] {
            let (grid, warnings) =
                known(resolve_grid(&unheard_hint(sidecar), &track, 0.0, 60.0, 0.0).unwrap());
            assert_eq!(grid.bpm, sidecar);
            assert_eq!(grid.tempo_source, Resolved::Sidecar);
            assert_eq!(grid.phase_source, Resolved::Inferred);
            assert_eq!(warnings.len(), 1, "{warnings:?}");
            assert!(
                warnings[0].starts_with("phase inferred from the audio"),
                "{warnings:?}"
            );
        }
    }

    #[test]
    fn unheard_tempo_kept_an_octave_from_the_heard_one() {
        // the heard tempo is folded into 60..=180: a sidecar tempo an octave away agrees with it
        for bpm in [200.0, 50.0] {
            let track = melody(bpm, 0.5, 8);
            let (heard, _) = infer_tempo(&track).unwrap();
            let ratio = (heard / bpm).log2();
            assert!(
                ratio.round() != 0.0 && (ratio - ratio.round()).abs() < 0.05,
                "{bpm} bpm heard as {heard}: the case needs an octave error"
            );
            let (grid, warnings) =
                known(resolve_grid(&unheard_hint(bpm), &track, 0.0, 60.0, 0.0).unwrap());
            assert_eq!(grid.bpm, bpm, "{warnings:?}");
            assert_eq!(grid.tempo_source, Resolved::Sidecar);
            assert!(
                !warnings
                    .iter()
                    .any(|w| w.starts_with("the take did not follow")),
                "{warnings:?}"
            );
        }
        // half or double time, within 4% of it
        assert!(tempo_agrees(160.0, 80.0) && tempo_agrees(100.0, 200.0));
        assert!(tempo_agrees(103.9, 200.0) && !tempo_agrees(104.1, 200.0));
        assert!(tempo_agrees(120.0, 120.0) && !tempo_agrees(125.0, 120.0));
        assert!(!tempo_agrees(150.0, 100.0) && !tempo_agrees(100.0, f64::NAN));
        // an octave off and beyond 4%: replaced
        let track = melody(200.0, 0.5, 8);
        let (grid, _) = known(resolve_grid(&unheard_hint(180.0), &track, 0.0, 60.0, 0.0).unwrap());
        assert_eq!(grid.tempo_source, Resolved::Inferred);
    }

    #[test]
    fn unheard_tempo_replaced_beyond_4_percent() {
        let track = melody(120.0, 0.5, 8);
        let (heard, _) = infer_tempo(&track).unwrap();
        let (grid, warnings) =
            known(resolve_grid(&unheard_hint(100.0), &track, 0.0, 60.0, 0.0).unwrap());
        assert_eq!(grid.bpm, heard);
        assert_eq!(grid.tempo_source, Resolved::Inferred);
        assert_eq!(grid.phase_source, Resolved::Inferred);
        assert!((grid.origin_seconds - 0.5).abs() <= 0.025, "{grid:?}");
        assert_eq!(
            warnings[0],
            format!(
                "the take did not follow the song's tempo (100 bpm, heard {} bpm)",
                trim_number(heard)
            )
        );
        assert!(warnings[1].starts_with("tempo and phase inferred from the audio ("));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }

    #[test]
    fn fewer_than_4_onsets_is_unresolved() {
        let three = onsets_at(&[(0.5, 1.0), (1.0, 1.0), (1.5, 1.0)], 3.0);
        let hints = [
            GridHint::default(),
            GridHint {
                bpm: Some(Hinted::sidecar(120.0)),
                ..GridHint::default()
            },
            unheard_hint(120.0),
        ];
        for hint in hints {
            assert_eq!(
                resolve_grid(&hint, &three, 0.0, 60.0, 0.0).unwrap(),
                GridOutcome::Unresolved(vec![UNRESOLVED_WARNING.to_string()]),
                "{hint:?}"
            );
        }
        assert_eq!(
            UNRESOLVED_WARNING,
            "too few onsets to infer a grid; notes are not quantized. Pass --bpm (and --bar1)"
        );
        assert_eq!(infer_tempo(&three), None);
        assert_eq!(infer_phase(&three, 120.0, 4.0), None);
        // an unheard sidecar tempo is kept when there is too little to check it, so a flag bar 1
        // gives a grid
        let flag_origin = GridHint {
            origin_seconds: Some(Hinted::flag(0.5)),
            ..unheard_hint(120.0)
        };
        let (grid, warnings) = known(resolve_grid(&flag_origin, &three, 0.0, 60.0, 0.0).unwrap());
        assert_eq!((grid.bpm, grid.origin_seconds), (120.0, 0.5));
        assert_eq!(
            (grid.tempo_source, grid.phase_source, grid.confidence),
            (Resolved::Sidecar, Resolved::Flag, None)
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        // a fully known grid needs no onsets
        assert!(matches!(
            resolve_grid(&sidecar_hint(), &three, 0.0, 60.0, 0.0),
            Ok(GridOutcome::Known(..))
        ));
    }

    #[test]
    fn shift_moves_origin_later() {
        let (grid, _) =
            known(resolve_grid(&sidecar_hint(), &empty_onsets(), 0.0, 30.0, 40.0).unwrap());
        assert!((grid.origin_seconds - 2.44).abs() < 1e-12, "{grid:?}");
        let (grid, _) =
            known(resolve_grid(&sidecar_hint(), &empty_onsets(), 0.0, 30.0, -100.0).unwrap());
        assert!((grid.origin_seconds - 2.3).abs() < 1e-12, "{grid:?}");
        // an inferred origin moves too
        let track = melody(120.0, 1.0, 4);
        let hint = GridHint {
            bpm: Some(Hinted::sidecar(120.0)),
            beats_per_bar: Some(Hinted::sidecar(4.0)),
            ..GridHint::default()
        };
        let (plain, _) = known(resolve_grid(&hint, &track, 0.0, 30.0, 0.0).unwrap());
        let (shifted, _) = known(resolve_grid(&hint, &track, 0.0, 30.0, 25.0).unwrap());
        assert!((shifted.origin_seconds - plain.origin_seconds - 0.025).abs() < 1e-12);
    }

    #[test]
    fn bar1_past_end_errors() {
        let hint = GridHint {
            origin_seconds: Some(Hinted::sidecar(14.0)),
            ..sidecar_hint()
        };
        let err = resolve_grid(&hint, &empty_onsets(), 0.0, 12.4, 0.0).unwrap_err();
        assert_eq!(
            err.to_string(),
            "bar 1 (14.000 s, from the sidecar) is past the end of the audio (12.400 s)"
        );
        assert_eq!(err.exit_code(), 1);
        let flag = GridHint {
            origin_seconds: Some(Hinted::flag(14.0)),
            ..sidecar_hint()
        };
        let err = resolve_grid(&flag, &empty_onsets(), 0.0, 12.4, 0.0).unwrap_err();
        assert_eq!(
            err.to_string(),
            "bar 1 (14.000 s, from --bar1) is past the end of the audio (12.400 s)"
        );
        // an unheard sidecar's bar 1 is ignored, so it is never past the end
        let track = melody(120.0, 0.5, 4);
        let unheard = GridHint {
            origin_seconds: Some(Hinted::sidecar(99.0)),
            ..unheard_hint(120.0)
        };
        let (grid, _) = known(resolve_grid(&unheard, &track, 0.0, 12.4, 0.0).unwrap());
        assert_eq!(grid.phase_source, Resolved::Inferred);
        // at the end exactly is not past it
        let hint = GridHint {
            origin_seconds: Some(Hinted::sidecar(12.4)),
            ..sidecar_hint()
        };
        assert!(resolve_grid(&hint, &empty_onsets(), 0.0, 12.4, 0.0).is_ok());
    }

    #[test]
    fn meter_assumed_when_unknown() {
        let hint = GridHint {
            beats_per_bar: None,
            ..sidecar_hint()
        };
        let (grid, warnings) = known(resolve_grid(&hint, &empty_onsets(), 0.0, 30.0, 0.0).unwrap());
        assert_eq!(grid.beats_per_bar, 4.0);
        assert_eq!(
            warnings,
            vec!["meter assumed 4/4; pass --meter".to_string()]
        );
    }

    #[test]
    fn invalid_hint_values_count_as_absent() {
        let hint = GridHint {
            bpm: Some(Hinted::sidecar(f64::NAN)),
            beats_per_bar: Some(Hinted::sidecar(-3.0)),
            origin_seconds: Some(Hinted::flag(f64::INFINITY)),
            meter: Some("4/4".into()),
            tempo_unheard: true,
        };
        assert_eq!(
            hint.validated(),
            GridHint {
                tempo_unheard: true,
                ..GridHint::default()
            }
        );
        let zero = GridHint {
            bpm: Some(Hinted::flag(0.0)),
            beats_per_bar: Some(Hinted::sidecar(f64::INFINITY)),
            ..sidecar_hint()
        };
        assert_eq!(
            zero.validated(),
            GridHint {
                bpm: None,
                beats_per_bar: None,
                ..sidecar_hint()
            }
        );
        let good = GridHint {
            meter: Some("6/8".into()),
            ..sidecar_hint()
        };
        assert_eq!(good.validated(), good);
        assert_eq!(
            resolve_grid(&hint.validated(), &empty_onsets(), 0.0, 30.0, 0.0).unwrap(),
            GridOutcome::Unresolved(vec![UNRESOLVED_WARNING.to_string()])
        );
    }

    #[test]
    fn bar_span_is_seconds_of_whole_bars() {
        // 100 bpm 4/4 from 2.4 s: a bar is 2.4 s
        let (start, end) = sidecar_hint().bar_span(2, 3, 0.0).unwrap();
        assert!((start - 4.8).abs() < 1e-12 && (end - 9.6).abs() < 1e-12);
        assert_eq!(sidecar_hint().bar_span(1, 1, 0.0), Some((2.4, 4.8)));
        // the shift moves the span with the grid's origin
        let (start, end) = sidecar_hint().bar_span(2, 3, 40.0).unwrap();
        assert!((start - 4.84).abs() < 1e-12 && (end - 9.64).abs() < 1e-12);
        let (grid, _) =
            known(resolve_grid(&sidecar_hint(), &empty_onsets(), 0.0, 30.0, 40.0).unwrap());
        assert!((start - (grid.origin_seconds + 2.4)).abs() < 1e-12);
        let (start, _) = sidecar_hint().bar_span(1, 1, -100.0).unwrap();
        assert!((start - 2.3).abs() < 1e-12);
        assert_eq!(sidecar_hint().bar_span(1, 1, f64::INFINITY), None);
        for hint in [
            GridHint {
                bpm: None,
                ..sidecar_hint()
            },
            GridHint {
                beats_per_bar: Some(Hinted::flag(0.0)),
                ..sidecar_hint()
            },
            GridHint {
                origin_seconds: None,
                ..sidecar_hint()
            },
        ] {
            assert_eq!(hint.bar_span(1, 2, 0.0), None, "{hint:?}");
        }
    }

    #[test]
    fn bar_span_needs_a_grid_resolve_grid_keeps() {
        // an unheard sidecar tempo is checked against the audio and its bar 1 ignored, so the
        // sidecar gives no grid to cut bars from
        let unheard = GridHint {
            origin_seconds: Some(Hinted::sidecar(2.4)),
            ..unheard_hint(100.0)
        };
        assert_eq!(unheard.bar_span(1, 2, 0.0), None);
        // a flag bar 1 alone does not settle the tempo
        let flag_origin = GridHint {
            origin_seconds: Some(Hinted::flag(2.4)),
            ..unheard.clone()
        };
        assert_eq!(flag_origin.bar_span(1, 2, 0.0), None);
        // a flag tempo is never second-guessed, and the sidecar's bar 1 is then used
        let flag_bpm = GridHint {
            bpm: Some(Hinted::flag(100.0)),
            ..unheard
        };
        assert_eq!(flag_bpm.bar_span(1, 1, 0.0), Some((2.4, 4.8)));
        let (grid, _) = known(resolve_grid(&flag_bpm, &empty_onsets(), 0.0, 30.0, 0.0).unwrap());
        assert_eq!(grid.origin_seconds, 2.4);
    }

    #[test]
    fn tempo_only_inferred_keeps_a_flag_origin() {
        let track = melody(120.0, 0.5, 8);
        let hint = GridHint {
            beats_per_bar: Some(Hinted::sidecar(4.0)),
            origin_seconds: Some(Hinted::flag(0.5)),
            ..GridHint::default()
        };
        let (grid, warnings) = known(resolve_grid(&hint, &track, 0.0, 60.0, 0.0).unwrap());
        assert_eq!(grid.origin_seconds, 0.5);
        assert_eq!(grid.phase_source, Resolved::Flag);
        assert_eq!(grid.tempo_source, Resolved::Inferred);
        let conf = grid.confidence.unwrap();
        assert_eq!(
            warnings,
            vec![format!(
                "tempo inferred from the audio ({:.1} bpm, confidence {conf:.2}); pass --bpm if you know it",
                grid.bpm
            )]
        );
    }

    #[test]
    fn degenerate_tracks_infer_nothing() {
        let mut flat = onsets_at(&[(0.5, 1.0), (1.0, 1.0), (1.5, 1.0), (2.0, 1.0)], 3.0);
        flat.strength.iter_mut().for_each(|v| *v = 0.5);
        assert_eq!(infer_tempo(&flat), None);
        let mut bad_hop = melody(120.0, 0.5, 2);
        bad_hop.hop_seconds = 0.0;
        assert_eq!(infer_tempo(&bad_hop), None);
        let track = melody(120.0, 0.5, 2);
        assert_eq!(infer_phase(&track, 0.0, 4.0), None);
        assert_eq!(infer_phase(&track, f64::NAN, 4.0), None);
        assert_eq!(infer_phase(&track, 120.0, 0.0), None);
        let mut zero = track.clone();
        zero.peaks.iter_mut().for_each(|p| p.strength = 0.0);
        assert_eq!(infer_phase(&zero, 120.0, 4.0), None);
    }

    #[test]
    fn meter_flag_parses_6_8() {
        assert_eq!(parse_meter("6/8").unwrap(), 3.0);
        assert_eq!(parse_meter("4/4").unwrap(), 4.0);
        assert_eq!(parse_meter("3/4").unwrap(), 3.0);
        assert_eq!(parse_meter("3/8").unwrap(), 1.5);
        assert_eq!(parse_meter("7/16").unwrap(), 1.75);
        assert_eq!(parse_meter("2/2").unwrap(), 4.0);
        assert_eq!(parse_meter("1/1").unwrap(), 4.0);
        assert_eq!(meter_label(4.0), "4/4");
        assert_eq!(meter_label(3.0), "3/4");
        assert_eq!(meter_label(1.5), "3/8");
        assert_eq!(meter_label(3.5), "7/8");
        assert_eq!(meter_label(1.75), "7/16");
    }

    #[test]
    fn bad_meter_rejected() {
        for bad in [
            "",
            "4",
            "4/",
            "/4",
            "0/4",
            "4/3",
            "4/32",
            "-3/4",
            "3.5/4",
            " 4/4",
            "4/4/4",
            "a/b",
            "99999999999/4",
        ] {
            let err = parse_meter(bad).unwrap_err();
            assert_eq!(
                err.to_string(),
                format!("bad --meter \"{bad}\"; use N/D such as 4/4 or 6/8"),
                "{bad:?}"
            );
            assert_eq!(err.exit_code(), 4, "{bad:?}");
        }
    }
}
