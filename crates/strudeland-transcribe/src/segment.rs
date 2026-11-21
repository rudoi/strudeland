//! Notes from a pitch track and onsets.
//!
//! Boundaries, in order:
//!
//! 1. Voicing: each run of voiced frames is a region; unvoiced gaps shorter than `GAP_BRIDGE_MS`
//!    are bridged (the decoded HMM already has the voicing hysteresis).
//! 2. Sustained pitch changes: inside a region, on the `MEDIAN_FRAMES`-smoothed MIDI of its
//!    voiced frames, a new note starts at the first frame of `PITCH_CHANGE_MS` that all lie more
//!    than `PITCH_CHANGE_ST` from the current note's reference, on the same side. The reference is
//!    the median of the note's smoothed frames so far, without its first `TUNING_SKIP_MS` (the
//!    attack), and needs `PITCH_CHANGE_MS` of them, so an attack glide does not split a note.
//!    Neighbours whose whole-note pitches then round to the same semitone, or lie within
//!    `PITCH_CHANGE_ST`, are joined again (a vibrato extreme is not a change). A change is seen
//!    only once it has held, so each boundary then moves back to the onset peak (at or above
//!    `ONSET_SPLIT_MIN`) nearest it within `PITCH_SNAP_MS` before it.
//! 3. Glides: a piece shorter than `GLIDE_MAX_MS` that glides into the next one (an attack glide
//!    or a portamento that a pitch change cut off) is merged into it.
//! 4. Drift: adjacent pieces less than `DRIFT_MERGE_ST` apart, the shorter under `DRIFT_MAX_MS`,
//!    with no re-articulation at the boundary (the loudness dip of step 5) and no onset peak of
//!    `DRIFT_ONSET_VETO` within a frame of it are one drifting note: a hummed pitch drifts most
//!    of a semitone without re-singing.
//! 5. Onsets: an onset peak at or above `ONSET_SPLIT_MIN` splits a note when both parts are at
//!    least `MIN_NOTE_MS` long and the loudness dips there (a re-articulation: a fast dip to
//!    `REARTICULATION_DIP` of the level within `REARTICULATION_LEVEL_MS`, or a slow one to
//!    `REARTICULATION_SLOW_DIP` within `REARTICULATION_SLOW_MS`), so repeated notes on one pitch
//!    separate while a hummed note's loudness wobble does not re-attack it.
//!
//! Notes shorter than `MIN_NOTE_MS` merge into an adjacent neighbour within `PITCH_CHANGE_ST`
//! (the closer pitch, the earlier on a tie) or are dropped. A note's pitch is the median of its
//! voiced frames after the first `TUNING_SKIP_MS`, without frames still in a glide (more than
//! `PITCH_CHANGE_ST` from a first median; a vibrato stays inside that and averages out), with
//! the tuning subtracted. A piece needs at least one pitched frame with credible voicing.
//!
//! Confidence is the weighted geometric mean `voiced^CONF_W_VOICED * stability^CONF_W_STABILITY *
//! fit^CONF_W_FIT`, multiplied by `0.5 + 0.5 * onset` for a note that starts at an onset split:
//!
//! - `voiced`: the mean voiced probability over the note's frames, with unpitched frames
//!   and frames below `MIN_VOICED_PROB_EMIT` counting 0;
//! - `stability`: `1 - min(MAD / STABILITY_FULL_ST, 1)`, the median absolute deviation in
//!   semitones of the pitch frames around the note's pitch (attack skipped, glides kept);
//! - `fit`: `1 - |cents| / 50`, the distance of the pitch from its nearest semitone.

use std::ops::Range;

use crate::consts::{
    CONF_W_FIT, CONF_W_STABILITY, CONF_W_VOICED, DRIFT_MAX_MS, DRIFT_MERGE_ST, DRIFT_ONSET_VETO,
    GAP_BRIDGE_MS, GLIDE_INSIDE, GLIDE_MAX_MS, MIN_NOTE_MS, MIN_VOICED_PROB_EMIT, ONSET_SPLIT_MIN,
    PITCH_CHANGE_MS, PITCH_CHANGE_ST, PITCH_SNAP_MS, REARTICULATION_DIP, REARTICULATION_LEVEL_MS,
    REARTICULATION_LOOK_MS, REARTICULATION_SLOW_DIP, REARTICULATION_SLOW_MS, STABILITY_FULL_ST,
    TUNING_SKIP_MS,
};
use crate::onset::OnsetTrack;
use crate::pitch::PitchTrack;
use crate::report::Components;
use crate::util::{
    clamp01, frame_midi, hop_of, median, median_smooth, median_sorted, ms_to_frames, voiced_runs,
};

/// A note before quantizing.
#[derive(Debug, Clone, PartialEq)]
pub struct RawNote {
    /// Seconds from the start of the analysed samples.
    pub start: f64,
    /// Seconds from the start of the analysed samples.
    pub end: f64,
    /// The pitch with the tuning subtracted, fractional MIDI.
    pub midi: f32,
    pub components: Components,
    /// 0..=1.
    pub confidence: f32,
    /// True when the note starts at an onset that split a stable voiced region.
    pub split_by_onset: bool,
}

/// A note in frames while boundaries are placed: frames `range`, and the onset strength when it
/// starts at an onset split.
#[derive(Debug, Clone, PartialEq)]
struct Piece {
    range: Range<usize>,
    onset: Option<f32>,
}

/// Per-frame facts the stages share: the tuned MIDI of voiced frames and, within each region,
/// their smoothed MIDI.
struct Frames {
    midi: Vec<Option<f32>>,
    smoothed: Vec<Option<f32>>,
}

/// Notes of `track`, split by the peaks of `onsets`, with `tuning_cents` (positive is sharp)
/// subtracted from their pitch. The notes are ordered and do not overlap.
pub fn segment(track: &PitchTrack, onsets: &OnsetTrack, tuning_cents: f32) -> Vec<RawNote> {
    let Some(hop) = hop_of(track) else {
        return Vec::new();
    };
    let shift = if tuning_cents.is_finite() {
        tuning_cents / 100.0
    } else {
        0.0
    };
    let regions = bridge(voiced_runs(track), ms_to_frames(GAP_BRIDGE_MS, hop));
    let frames = analyse(track, &regions, shift);
    let skip = ms_to_frames(TUNING_SKIP_MS, hop);

    let mut pieces = Vec::new();
    for region in &regions {
        let changes = pitch_changes(region, &frames, hop, skip);
        pieces.extend(snap_to_onsets(changes, onsets, hop));
    }
    let pieces = merge_glides(pieces, &frames, ms_to_frames(GLIDE_MAX_MS, hop), skip);
    let pieces = merge_drift(pieces, &frames, onsets, hop, skip);
    let pieces = split_at_onsets(pieces, onsets, hop);
    let pieces = merge_short(pieces, &frames, ms_to_frames(MIN_NOTE_MS, hop), skip);
    pieces
        .iter()
        .filter_map(|p| note_of(p, track, &frames, hop, skip))
        .collect()
}

/// Voiced runs with the gaps shorter than `bridge_frames` closed.
fn bridge(runs: Vec<Range<usize>>, bridge_frames: usize) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::with_capacity(runs.len());
    for run in runs {
        match out.last_mut() {
            Some(last) if run.start - last.end < bridge_frames => last.end = run.end,
            _ => out.push(run),
        }
    }
    out
}

/// The tuned MIDI of every frame, and within each region the smoothed MIDI of its voiced frames
/// (unvoiced frames of a bridged gap are skipped, not smoothed over).
fn analyse(track: &PitchTrack, regions: &[Range<usize>], shift: f32) -> Frames {
    let n = track.frames.len();
    let midi: Vec<Option<f32>> = (0..n)
        .map(|i| frame_midi(track, i).map(|m| m - shift))
        .collect();
    let mut smoothed = vec![None; n];
    for region in regions {
        let idx: Vec<usize> = region.clone().filter(|&i| midi[i].is_some()).collect();
        let values: Vec<f32> = idx.iter().filter_map(|&i| midi[i]).collect();
        let smooth = median_smooth(&values);
        for (k, &i) in idx.iter().enumerate() {
            smoothed[i] = Some(smooth[k]);
        }
    }
    Frames { midi, smoothed }
}

/// The notes of one region, split at sustained pitch changes.
fn pitch_changes(region: &Range<usize>, frames: &Frames, hop: f64, skip: usize) -> Vec<Piece> {
    let hold = ms_to_frames(PITCH_CHANGE_MS, hop);
    let voiced: Vec<(usize, f32)> = region
        .clone()
        .filter_map(|i| frames.smoothed[i].map(|m| (i, m)))
        .collect();
    let mut starts = vec![region.start];
    // the reference: the sorted smoothed pitches of the current note after its attack
    let mut sorted: Vec<f32> = Vec::new();
    let mut note_start = 0usize;
    let mut j = 0usize;
    while j < voiced.len() {
        if sorted.len() >= hold && j + hold <= voiced.len() {
            let reference = median_sorted(&sorted);
            let window = &voiced[j..j + hold];
            let above = window.iter().all(|w| w.1 - reference > PITCH_CHANGE_ST);
            let below = window.iter().all(|w| reference - w.1 > PITCH_CHANGE_ST);
            if above || below {
                starts.push(voiced[j].0);
                note_start = j;
                sorted.clear();
            }
        }
        if j >= note_start + skip {
            let at = sorted.partition_point(|&v| v < voiced[j].1);
            sorted.insert(at, voiced[j].1);
        }
        j += 1;
    }
    let mut ends: Vec<usize> = starts[1..].to_vec();
    ends.push(region.end);
    let pieces = starts
        .into_iter()
        .zip(ends)
        .map(|(a, b)| Piece {
            range: a..b,
            onset: None,
        })
        .collect();
    merge_same_pitch(pieces, frames, skip)
}

/// Rejoins neighbours split by a pitch change whose pitches turn out the same: the same nearest
/// semitone, or within `PITCH_CHANGE_ST`. A reference taken early in a note can sit on a vibrato
/// extreme, so the other extreme looks like a change while the whole notes do not differ. The
/// closest pair merges first and the merged pitch is measured again.
fn merge_same_pitch(mut pieces: Vec<Piece>, frames: &Frames, skip: usize) -> Vec<Piece> {
    loop {
        let pitches: Vec<Option<f32>> = pieces
            .iter()
            .map(|p| note_pitch(&p.range, frames, skip))
            .collect();
        let closest = pitches
            .windows(2)
            .enumerate()
            .filter_map(|(k, w)| {
                let (a, b) = (w[0]?, w[1]?);
                let same = a.round() == b.round() || (a - b).abs() <= PITCH_CHANGE_ST;
                same.then_some((k, (a - b).abs()))
            })
            .min_by(|x, y| x.1.total_cmp(&y.1));
        let Some((k, _)) = closest else {
            return pieces;
        };
        let next = pieces.remove(k + 1);
        pieces[k].range.end = next.range.end;
    }
}

/// Moves each boundary between adjacent pieces of a region (a pitch change) back to the onset
/// peak at or above `ONSET_SPLIT_MIN` nearest it within `PITCH_SNAP_MS` before it (or one frame
/// after). A change is found only once the new pitch has held `PITCH_CHANGE_MS` beyond
/// `PITCH_CHANGE_ST`, which a vibrato or a portamento delays past the attack the onset function
/// sees. Each piece keeps at least one frame.
fn snap_to_onsets(mut pieces: Vec<Piece>, onsets: &OnsetTrack, hop: f64) -> Vec<Piece> {
    let before = ms_to_frames(PITCH_SNAP_MS, hop) as f64;
    for k in 1..pieces.len() {
        let at = pieces[k].range.start;
        if pieces[k - 1].range.end != at {
            continue;
        }
        let (lo, hi) = (pieces[k - 1].range.start + 1, pieces[k].range.end - 1);
        let snapped = onsets
            .peaks
            .iter()
            .filter(|o| o.strength >= ONSET_SPLIT_MIN && o.seconds.is_finite())
            .map(|o| (o.seconds / hop).round())
            .filter(|&f| f >= at as f64 - before && f <= at as f64 + 1.0)
            .filter(|&f| f >= lo as f64 && f <= hi as f64)
            .min_by(|a, b| (a - at as f64).abs().total_cmp(&(b - at as f64).abs()));
        if let Some(f) = snapped {
            let f = f as usize;
            pieces[k - 1].range.end = f;
            pieces[k].range.start = f;
        }
    }
    pieces
}

/// Splits notes at onset peaks at or above `ONSET_SPLIT_MIN` that leave both parts at least
/// `MIN_NOTE_MS` long and re-articulate the note ([`rearticulated`]).
fn split_at_onsets(mut pieces: Vec<Piece>, onsets: &OnsetTrack, hop: f64) -> Vec<Piece> {
    let min = ms_to_frames(MIN_NOTE_MS, hop);
    for onset in &onsets.peaks {
        if !(onset.strength >= ONSET_SPLIT_MIN && onset.seconds.is_finite()) {
            continue;
        }
        let k = (onset.seconds / hop).round();
        if k < 0.0 {
            continue;
        }
        let k = k as usize;
        let Some(at) = pieces
            .iter()
            .position(|p| p.range.start + min <= k && k + min <= p.range.end)
        else {
            continue;
        };
        if !rearticulated(onsets, k, &pieces[at].range, hop) {
            continue;
        }
        let end = pieces[at].range.end;
        pieces[at].range.end = k;
        pieces.insert(
            at + 1,
            Piece {
                range: k..end,
                onset: Some(clamp01(onset.strength)),
            },
        );
    }
    pieces
}

/// True when the onset at frame `k` inside the note `range` re-articulates the note: a fast dip
/// in loudness ([`dip_ratio`] over `REARTICULATION_LEVEL_MS` at most `REARTICULATION_DIP`) or a
/// deep slow one (over `REARTICULATION_SLOW_MS`, at most `REARTICULATION_SLOW_DIP`). A repeated
/// note is re-sung, so its loudness falls between the two; a hummed note's loudness wobble
/// moves it a few dB, slower than a re-articulation and shallower than a slow one. A track
/// without RMS (or a valley at the note's edge) cannot show the dip and counts as
/// re-articulated.
fn rearticulated(onsets: &OnsetTrack, k: usize, range: &Range<usize>, hop: f64) -> bool {
    let fast = dip_ratio(
        onsets,
        k,
        range,
        ms_to_frames(REARTICULATION_LEVEL_MS, hop),
        hop,
    );
    let slow = dip_ratio(
        onsets,
        k,
        range,
        ms_to_frames(REARTICULATION_SLOW_MS, hop),
        hop,
    );
    match (fast, slow) {
        (Some(f), Some(s)) => f <= REARTICULATION_DIP || s <= REARTICULATION_SLOW_DIP,
        _ => true,
    }
}

/// The loudness dip at the onset at frame `k` inside `range`: the lowest `REARTICULATION_WIN`
/// RMS from `REARTICULATION_LOOK_MS` before `k` to one frame after, over the quieter side's
/// level (the highest RMS within `level` frames before and after that valley, inside the
/// note). `None` without RMS or when a side is empty or silent.
fn dip_ratio(
    onsets: &OnsetTrack,
    k: usize,
    range: &Range<usize>,
    level: usize,
    hop: f64,
) -> Option<f32> {
    let rms = &onsets.rms;
    if rms.len() < range.end || k >= range.end {
        return None;
    }
    let look = ms_to_frames(REARTICULATION_LOOK_MS, hop);
    let lo = k.saturating_sub(look).max(range.start);
    let hi = (k + 2).min(range.end);
    let (v, valley) = (lo..hi)
        .map(|i| (i, rms[i]))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    // an empty side (the valley at the note's edge) has no level: `None`
    let max_of = |r: Range<usize>| rms[r].iter().copied().reduce(f32::max);
    let before = max_of(v.saturating_sub(level).max(range.start)..v)?;
    let after = max_of(v + 1..(v + 1 + level).min(range.end))?;
    let side = before.min(after);
    (side.is_finite() && side > 0.0 && valley.is_finite()).then(|| valley / side)
}

/// The pitch of a note: the median of its voiced frames after the attack, then again over only
/// those within `PITCH_CHANGE_ST` of that first median (the frames not still in a glide; a
/// vibrato stays inside it, so it averages out). A note no longer than its attack uses all its
/// voiced frames.
fn note_pitch(range: &Range<usize>, frames: &Frames, skip: usize) -> Option<f32> {
    let values = spread_frames(range, frames, skip);
    let first = median(&values)?;
    let steady: Vec<f32> = values
        .into_iter()
        .filter(|m| (m - first).abs() <= PITCH_CHANGE_ST)
        .collect();
    median(&steady).or(Some(first))
}

/// The voiced frames of a note after its attack (all its voiced frames when the note is no
/// longer than the attack): the frames its stability is measured over.
fn spread_frames(range: &Range<usize>, frames: &Frames, skip: usize) -> Vec<f32> {
    let after: Vec<f32> = ((range.start + skip).min(range.end)..range.end)
        .filter_map(|i| frames.midi[i])
        .collect();
    if after.is_empty() {
        range.clone().filter_map(|i| frames.midi[i]).collect()
    } else {
        after
    }
}

/// Merges notes shorter than `min` frames into an adjacent neighbour within `PITCH_CHANGE_ST`,
/// or drops them.
fn merge_short(mut pieces: Vec<Piece>, frames: &Frames, min: usize, skip: usize) -> Vec<Piece> {
    let pitch = |p: &Piece| note_pitch(&p.range, frames, skip);
    loop {
        let Some(at) = pieces.iter().position(|p| p.range.len() < min) else {
            return pieces;
        };
        let here = pitch(&pieces[at]);
        let distance = |other: Option<&Piece>, adjacent: fn(&Piece, &Piece) -> bool| {
            let other = other.filter(|o| adjacent(o, &pieces[at]))?;
            let d = (pitch(other)? - here?).abs();
            (d <= PITCH_CHANGE_ST).then_some(d)
        };
        let before = at
            .checked_sub(1)
            .and_then(|b| distance(pieces.get(b), |o, p| o.range.end == p.range.start));
        let after = distance(pieces.get(at + 1), |o, p| p.range.end == o.range.start);
        let short = pieces.remove(at);
        let into_next = match (before, after) {
            (Some(b), Some(a)) => a < b,
            (None, Some(_)) => true,
            _ => false,
        };
        if into_next {
            pieces[at].range.start = short.range.start;
            pieces[at].onset = short.onset;
        } else if before.is_some() {
            pieces[at - 1].range.end = short.range.end;
        }
    }
}

/// Merges a glide into the note it leads to: a piece shorter than `max` frames that ends where
/// the next piece starts, whose smoothed pitch moves more than `PITCH_CHANGE_ST` toward the next
/// piece's pitch, ends within `PITCH_CHANGE_ST` of it, and passes through: the median of its
/// frames lies at least `GLIDE_INSIDE` of the way in from either end of its travel (a held note
/// whose end a late pitch change left in the piece has its median at the start). An attack glide or a portamento that
/// a pitch change cut off is the start of the note it reaches, not a note. It runs before the
/// onset splits, so a re-attack inside the merged note still splits it.
fn merge_glides(mut pieces: Vec<Piece>, frames: &Frames, max: usize, skip: usize) -> Vec<Piece> {
    let mut k = 0;
    while k + 1 < pieces.len() {
        let (a, b) = (&pieces[k], &pieces[k + 1]);
        let glide = a.range.len() < max
            && a.range.end == b.range.start
            && (|| {
                let target = note_pitch(&b.range, frames, skip)?;
                let voiced: Vec<f32> = a.range.clone().filter_map(|i| frames.smoothed[i]).collect();
                let (first, last) = (*voiced.first()?, *voiced.last()?);
                // a glide passes through: its median lies inside its travel, not at one end
                // (a held note that jumps at its end is not a glide)
                let middle = median(&voiced)?;
                let travel = last - first;
                let inside = (middle - first) / travel;
                if !(GLIDE_INSIDE..=1.0 - GLIDE_INSIDE).contains(&inside) {
                    return Some(false);
                }
                let toward = (target - first).signum() * (last - first);
                Some(toward > PITCH_CHANGE_ST && (target - last).abs() <= PITCH_CHANGE_ST)
            })()
            .unwrap_or(false);
        if glide {
            let a = pieces.remove(k);
            pieces[k].range.start = a.range.start;
            pieces[k].onset = a.onset;
        } else {
            k += 1;
        }
    }
    pieces
}

/// Rejoins a pitch-change split that is a drift, not a new note: adjacent pieces whose pitches
/// lie less than `DRIFT_MERGE_ST` apart, the shorter under `DRIFT_MAX_MS`, with no
/// re-articulation at the boundary ([`rearticulated`]: a hummed voice drifts a semitone's worth
/// without re-singing, while a new note dips in loudness) and no onset peak of
/// `DRIFT_ONSET_VETO` within a frame of it (a legato note change without a dip is still
/// attacked). The closest pair merges first and the merged pitch is measured again. A track
/// without RMS cannot show the dip, so nothing merges.
fn merge_drift(
    mut pieces: Vec<Piece>,
    frames: &Frames,
    onsets: &OnsetTrack,
    hop: f64,
    skip: usize,
) -> Vec<Piece> {
    let max = ms_to_frames(DRIFT_MAX_MS, hop);
    // one pitch per piece, measured again only for a merged piece
    let mut pitches: Vec<Option<f32>> = pieces
        .iter()
        .map(|p| note_pitch(&p.range, frames, skip))
        .collect();
    loop {
        let closest = (1..pieces.len())
            .filter_map(|k| {
                let (a, b) = (&pieces[k - 1], &pieces[k]);
                let d = (pitches[k - 1]? - pitches[k]?).abs();
                let drift = a.range.end == b.range.start
                    && d < DRIFT_MERGE_ST
                    && a.range.len().min(b.range.len()) < max
                    && !attacked(onsets, b.range.start, hop)
                    && !rearticulated(onsets, b.range.start, &(a.range.start..b.range.end), hop);
                drift.then_some((k, d))
            })
            .min_by(|x, y| x.1.total_cmp(&y.1));
        let Some((k, _)) = closest else {
            return pieces;
        };
        let next = pieces.remove(k);
        pitches.remove(k);
        pieces[k - 1].range.end = next.range.end;
        pitches[k - 1] = note_pitch(&pieces[k - 1].range, frames, skip);
    }
}

/// True when an onset peak of at least `DRIFT_ONSET_VETO` lies within one frame of frame `at`.
fn attacked(onsets: &OnsetTrack, at: usize, hop: f64) -> bool {
    onsets.peaks.iter().any(|o| {
        o.strength >= DRIFT_ONSET_VETO
            && o.seconds.is_finite()
            && ((o.seconds / hop).round() - at as f64).abs() <= 1.0
    })
}

/// The note of a piece; `None` when it has no pitched frame with credible voicing.
fn note_of(
    piece: &Piece,
    track: &PitchTrack,
    frames: &Frames,
    hop: f64,
    skip: usize,
) -> Option<RawNote> {
    let range = &piece.range;
    let voiced_sum = range
        .clone()
        .filter(|&i| frames.midi[i].is_some())
        .map(|i| clamp01(track.frames[i].voiced_prob))
        .filter(|&p| p >= MIN_VOICED_PROB_EMIT)
        .sum::<f32>();
    if voiced_sum == 0.0 {
        return None;
    }
    let voiced = clamp01(voiced_sum / range.len() as f32);
    let midi = note_pitch(range, frames, skip)?;
    let deviations: Vec<f32> = spread_frames(range, frames, skip)
        .iter()
        .map(|m| (m - midi).abs())
        .collect();
    let mad = median(&deviations).unwrap_or(0.0);
    let stability = clamp01(1.0 - (mad / STABILITY_FULL_ST).min(1.0));
    let fit = clamp01(1.0 - (midi - midi.round()).abs() * 100.0 / 50.0);
    let mut confidence =
        voiced.powf(CONF_W_VOICED) * stability.powf(CONF_W_STABILITY) * fit.powf(CONF_W_FIT);
    if let Some(s) = piece.onset {
        confidence *= 0.5 + 0.5 * s;
    }
    Some(RawNote {
        start: range.start as f64 * hop,
        end: range.end as f64 * hop,
        midi,
        components: Components {
            voiced,
            stability,
            fit,
            onset: piece.onset,
        },
        confidence: clamp01(confidence),
        split_by_onset: piece.onset.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consts::ANALYSIS_RATE;
    use crate::eval::synth::{Case, SynthSettings};
    use crate::onset::onset_strength;
    use crate::test_support::{audio, hop, run, sine, track, track_of, voice};
    use crate::tuning::estimate_tuning;

    /// No onsets: boundaries from voicing and pitch only.
    fn no_onsets() -> OnsetTrack {
        OnsetTrack {
            hop_seconds: hop(),
            strength: Vec::new(),
            rms: Vec::new(),
            peaks: Vec::new(),
        }
    }

    /// The notes of synth audio through the whole front end: pYIN, onsets, tuning, segment.
    fn transcribe(samples: &[f32]) -> Vec<RawNote> {
        let t = track(samples);
        segment(&t, &onset_strength(samples), estimate_tuning(&t))
    }

    /// Fails unless `notes` match `truth` one to one: the same rounded MIDI and a start within
    /// `tolerance` seconds.
    fn assert_matches(label: &str, notes: &[RawNote], truth: &[(f64, f64, i32)], tolerance: f64) {
        let summary: Vec<String> = notes
            .iter()
            .map(|n| format!("{:.3}-{:.3} {:.2}", n.start, n.end, n.midi))
            .collect();
        assert_eq!(notes.len(), truth.len(), "{label}: {summary:?}");
        for (n, t) in notes.iter().zip(truth) {
            assert_eq!(n.midi.round() as i32, t.2, "{label}: pitch in {summary:?}");
            assert!(
                (n.start - t.0).abs() <= tolerance,
                "{label}: start {:.3} for {:.3} in {summary:?}",
                n.start,
                t.0
            );
        }
    }

    fn frames(values: &[(Option<f32>, usize)]) -> Vec<Option<f32>> {
        values.iter().flat_map(|&(m, n)| run(m, n)).collect()
    }

    #[test]
    fn separates_notes_at_voicing_gaps() {
        let truth = [(0.2, 0.6, 60), (0.8, 1.2, 60), (1.4, 1.8, 64)];
        for (label, settings) in [("sine", sine()), ("voice", voice())] {
            let notes = transcribe(&audio(&truth, settings));
            assert_matches(label, &notes, &truth, 0.03);
            for pair in notes.windows(2) {
                assert!(pair[0].end < pair[1].start, "{label}: a gap stays a gap");
            }
            assert!(notes.iter().all(|n| !n.split_by_onset), "{label}");
        }
    }

    #[test]
    fn bridges_gaps_under_40ms() {
        // a 30 ms gap is bridged into one note
        let t = track_of(
            &frames(&[
                (None, 5),
                (Some(60.0), 30),
                (None, 3),
                (Some(60.0), 30),
                (None, 5),
            ]),
            0.9,
        );
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!((notes[0].start - 0.05).abs() < 1e-6 && (notes[0].end - 0.68).abs() < 1e-6);
        // a 40 ms gap is not
        let t = track_of(
            &frames(&[
                (None, 5),
                (Some(60.0), 30),
                (None, 4),
                (Some(60.0), 30),
                (None, 5),
            ]),
            0.9,
        );
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!((notes[0].end - 0.35).abs() < 1e-6 && (notes[1].start - 0.39).abs() < 1e-6);
        // a bridged gap's frames count as unvoiced in the voiced component
        let t = track_of(
            &frames(&[(Some(60.0), 20), (None, 2), (Some(60.0), 18)]),
            1.0,
        );
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 1);
        assert!(
            (notes[0].components.voiced - 0.95).abs() < 1e-6,
            "{notes:?}"
        );
    }

    #[test]
    fn splits_on_sustained_pitch_change() {
        // legato: no gap and no onset needed
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(60.7), 30)]), 0.9);
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 2, "0.7 st: {notes:?}");
        assert!((notes[0].end - 0.30).abs() < 1e-6 && (notes[1].start - 0.30).abs() < 1e-6);
        assert!((notes[1].midi - 60.7).abs() < 1e-3);
        // half a semitone is not a change
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(60.5), 30)]), 0.9);
        assert_eq!(segment(&t, &no_onsets(), 0.0).len(), 1, "0.5 st");
        // held 40 ms is not sustained
        let t = track_of(
            &frames(&[(Some(60.0), 30), (Some(62.0), 4), (Some(60.0), 30)]),
            0.9,
        );
        assert_eq!(segment(&t, &no_onsets(), 0.0).len(), 1, "40 ms excursion");
        // held 60 ms is
        let t = track_of(
            &frames(&[(Some(60.0), 30), (Some(62.0), 6), (Some(60.0), 30)]),
            0.9,
        );
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 3, "60 ms excursion: {notes:?}");
        // synth legato melody, onsets ignored
        let truth = [
            (0.2, 0.6, 60),
            (0.6, 1.0, 62),
            (1.0, 1.4, 59),
            (1.4, 1.9, 60),
        ];
        let samples = audio(&truth, voice());
        let t = track(&samples);
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_matches("legato voice", &notes, &truth, 0.03);
    }

    #[test]
    fn ignores_vibrato_and_short_glide() {
        let settings = SynthSettings {
            vibrato_cents: 50.0,
            attack_glide_ms: 60.0,
            ..voice()
        };
        let truth = [(0.2, 1.4, 64)];
        let notes = transcribe(&audio(&truth, settings.clone()));
        assert_matches("long vibrato note", &notes, &truth, 0.03);
        assert!((notes[0].midi - 64.0).abs() <= 0.15, "{notes:?}");
        // a melody: every note once, vibrato never splits
        let truth = [
            (0.2, 0.6, 60),
            (0.6, 1.2, 64),
            (1.2, 1.5, 62),
            (1.7, 2.6, 67),
            (2.6, 2.9, 65),
        ];
        let notes = transcribe(&audio(&truth, settings));
        assert_matches("vibrato melody", &notes, &truth, 0.03);
        // portamento between notes is a glide, not a note
        let settings = SynthSettings {
            portamento_ms: 60.0,
            ..voice()
        };
        let notes = transcribe(&audio(&truth, settings));
        assert_matches("portamento melody", &notes, &truth, 0.04);
    }

    #[test]
    fn repeated_pitch_splits_on_onset() {
        let truth = [
            (0.2, 0.5, 67),
            (0.5, 0.8, 67),
            (0.8, 1.0, 67),
            (1.0, 1.5, 67),
        ];
        let samples = audio(&truth, voice());
        let notes = transcribe(&samples);
        assert_matches("repeats", &notes, &truth, 0.03);
        assert!(!notes[0].split_by_onset && notes[0].components.onset.is_none());
        for n in &notes[1..] {
            assert!(n.split_by_onset, "{n:?}");
            let s = n.components.onset.expect("onset component");
            assert!((ONSET_SPLIT_MIN..=1.0).contains(&s), "{n:?}");
        }
        // without onsets the repeats are one note
        let t = track(&samples);
        assert_eq!(segment(&t, &no_onsets(), 0.0).len(), 1);
        // a weak onset does not split
        let weak = OnsetTrack {
            hop_seconds: hop(),
            strength: Vec::new(),
            rms: Vec::new(),
            peaks: vec![crate::onset::Onset {
                seconds: 0.5,
                strength: ONSET_SPLIT_MIN - 0.01,
            }],
        };
        assert_eq!(segment(&t, &weak, 0.0).len(), 1);
        // nor one that would leave a part under 60 ms
        let early = OnsetTrack {
            peaks: vec![crate::onset::Onset {
                seconds: notes[0].start + 0.05,
                strength: 1.0,
            }],
            ..weak
        };
        assert_eq!(segment(&t, &early, 0.0).len(), 1);
    }

    #[test]
    fn a_held_note_splits_only_where_its_loudness_dips() {
        // a held 60 over 1 s with a strong onset peak at 0.5 s
        let t = track_of(&run(Some(60.0), 100), 0.9);
        let with_rms = |rms: Vec<f32>| OnsetTrack {
            rms,
            ..peaks(&[0.5], 0.8)
        };
        // the loudness wobbles +-2 dB without a dip: one note
        let wobble: Vec<f32> = (0..100)
            .map(|i| 0.1 * 10f32.powf(2.0 * (i as f32 * 0.4).sin() / 20.0))
            .collect();
        assert_eq!(segment(&t, &with_rms(wobble.clone()), 0.0).len(), 1);
        // a fast dip to half the level just before the onset: two notes
        let mut fast = wobble.clone();
        for (i, v) in fast.iter_mut().enumerate().take(51).skip(48) {
            *v *= if i == 49 { 0.5 } else { 0.8 };
        }
        let notes = segment(&t, &with_rms(fast), 0.0);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!((notes[1].start - 0.5).abs() < 1e-6 && notes[1].split_by_onset);
        // a slow fade to 0.4 and back over 200 ms: two notes
        let slow: Vec<f32> = (0..100)
            .map(|i| {
                let d = (i as f32 - 49.0).abs();
                0.1 * if d < 10.0 { 0.4 + 0.06 * d } else { 1.0 }
            })
            .collect();
        assert_eq!(segment(&t, &with_rms(slow), 0.0).len(), 2);
        // the same slow fade only to 0.6: one note
        let shallow: Vec<f32> = (0..100)
            .map(|i| {
                let d = (i as f32 - 49.0).abs();
                0.1 * if d < 10.0 { 0.6 + 0.04 * d } else { 1.0 }
            })
            .collect();
        assert_eq!(segment(&t, &with_rms(shallow), 0.0).len(), 1);
        // without RMS the onset splits as before
        assert_eq!(segment(&t, &peaks(&[0.5], 0.8), 0.0).len(), 2);
    }

    #[test]
    fn a_drift_without_a_re_articulation_stays_one_note() {
        // 300 ms on 60, then 120 ms drifting up to 60.9: a pitch change, rounding to 61
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(60.9), 12)]), 0.9);
        let level = |n: usize| OnsetTrack {
            rms: vec![0.1; n],
            ..no_onsets()
        };
        assert_eq!(segment(&t, &no_onsets(), 0.0).len(), 2, "no RMS: two notes");
        let notes = segment(&t, &level(42), 0.0);
        assert_eq!(notes.len(), 1, "a steady level: one note {notes:?}");
        assert_eq!(notes[0].midi.round(), 60.0);
        // a dip at the boundary is a new note
        let mut dip = vec![0.1f32; 42];
        dip[29] = 0.05;
        let dipped = OnsetTrack {
            rms: dip,
            ..no_onsets()
        };
        assert_eq!(segment(&t, &dipped, 0.0).len(), 2, "re-articulated");
        // more than a semitone apart, or both pieces 200 ms or longer: two notes
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(61.1), 12)]), 0.9);
        assert_eq!(segment(&t, &level(42), 0.0).len(), 2, "1.1 st");
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(60.9), 20)]), 0.9);
        assert_eq!(segment(&t, &level(50), 0.0).len(), 2, "200 ms each");
    }

    #[test]
    fn a_legato_half_step_with_an_attack_is_not_a_drift() {
        // 300 ms on 64, then a legato 120 ms on 64.9 (an f4 sung flat): no loudness dip
        let t = track_of(&frames(&[(Some(64.0), 30), (Some(64.9), 12)]), 0.9);
        let flat = |seconds: &[f64], strength: f32| OnsetTrack {
            rms: vec![0.1; 42],
            ..peaks(seconds, strength)
        };
        let rounded = |o: &OnsetTrack| -> Vec<f32> {
            segment(&t, o, 0.0).iter().map(|n| n.midi.round()).collect()
        };
        // attacked at the change: two notes
        assert_eq!(rounded(&flat(&[0.3], 0.8)), [64.0, 65.0]);
        assert_eq!(rounded(&flat(&[0.31], 0.8)), [64.0, 65.0], "a frame late");
        // a hummed wobble's weaker peak, or none: one drifting note
        assert_eq!(rounded(&flat(&[0.3], 0.5)), [64.0]);
        assert_eq!(rounded(&flat(&[], 0.0)), [64.0]);
        // a whole semitone apart is a note, not a drift
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(61.0), 12)]), 0.9);
        let notes = segment(&t, &flat(&[], 0.0), 0.0);
        assert_eq!(notes.len(), 2, "1.0 st {notes:?}");
    }

    #[test]
    fn hummed_wobble_does_not_split_but_a_re_sung_note_does() {
        let hummed = SynthSettings {
            attack_ms: 60.0,
            release_ms: 30.0,
            amp_wobble_db: 3.0,
            ..voice()
        };
        // one long note under a +-3 dB loudness wobble
        let truth = [(0.2, 2.2, 50)];
        let notes = transcribe(&audio(&truth, hummed.clone()));
        assert_matches("wobbled note", &notes, &truth, 0.05);
        // the same pitch re-sung, soft attacks and all
        let truth = [
            (0.2, 0.7, 50),
            (0.7, 1.2, 50),
            (1.2, 1.7, 50),
            (1.7, 2.2, 52),
        ];
        let notes = transcribe(&audio(&truth, hummed));
        assert_matches("re-sung notes", &notes, &truth, 0.05);
    }

    /// `rms` with onset peaks at `seconds` (strength 0.8).
    fn levels(rms: Vec<f32>, seconds: &[f64]) -> OnsetTrack {
        OnsetTrack {
            rms,
            ..peaks(seconds, 0.8)
        }
    }

    /// Frames with these MIDI values, smoothed the same.
    fn frames_of(midi: Vec<Option<f32>>) -> Frames {
        Frames {
            smoothed: midi.clone(),
            midi,
        }
    }

    #[test]
    fn a_valley_at_the_note_edge_counts_as_re_articulated() {
        let h = f64::from(hop());
        // the lowest level of the look-back is the note's first frame: no level before it
        let mut rms = vec![0.1f32; 40];
        rms[0] = 0.09;
        let onsets = levels(rms, &[0.06]);
        assert_eq!(dip_ratio(&onsets, 6, &(0..40), 3, h), None);
        assert!(rearticulated(&onsets, 6, &(0..40), h));
        let note = || {
            vec![Piece {
                range: 0..40,
                onset: None,
            }]
        };
        assert_eq!(split_at_onsets(note(), &onsets, h).len(), 2, "split");
        // the same level a frame later has a side each way: a 0.9 dip is no re-articulation
        let mut rms = vec![0.1f32; 40];
        rms[1] = 0.09;
        let onsets = levels(rms, &[0.06]);
        assert!(dip_ratio(&onsets, 6, &(0..40), 3, h).is_some_and(|r| (r - 0.9).abs() < 1e-6));
        assert_eq!(split_at_onsets(note(), &onsets, h).len(), 1, "no split");

        // a drift whose valley is the joined note's first or last frame is not rejoined
        let pair = |split: usize| {
            vec![
                Piece {
                    range: 0..split,
                    onset: None,
                },
                Piece {
                    range: split..40,
                    onset: None,
                },
            ]
        };
        let drift = |split: usize| {
            frames_of(
                (0..40)
                    .map(|i| Some(if i < split { 60.0 } else { 60.8 }))
                    .collect(),
            )
        };
        let skip = ms_to_frames(TUNING_SKIP_MS, h);
        let quiet = |at: usize| {
            let mut rms = vec![0.1f32; 40];
            rms[at] = 0.09;
            levels(rms, &[])
        };
        // the first piece is 4 frames, the valley at frame 0
        let merged = merge_drift(pair(4), &drift(4), &quiet(0), h, skip);
        assert_eq!(merged.len(), 2, "left edge: {merged:?}");
        assert_eq!(merge_drift(pair(4), &drift(4), &quiet(1), h, skip).len(), 1);
        // the second piece is 1 frame, the valley at frame 39
        let merged = merge_drift(pair(39), &drift(39), &quiet(39), h, skip);
        assert_eq!(merged.len(), 2, "right edge: {merged:?}");
        assert_eq!(
            merge_drift(pair(38), &drift(38), &quiet(38), h, skip).len(),
            1
        );
    }

    /// Onset peaks at `seconds`, each of `strength`.
    fn peaks(seconds: &[f64], strength: f32) -> OnsetTrack {
        OnsetTrack {
            hop_seconds: hop(),
            strength: Vec::new(),
            rms: Vec::new(),
            peaks: seconds
                .iter()
                .map(|&seconds| crate::onset::Onset { seconds, strength })
                .collect(),
        }
    }

    #[test]
    fn pitch_change_snaps_back_to_its_onset() {
        let piece = |r: Range<usize>| Piece {
            range: r,
            onset: None,
        };
        let pieces = || vec![piece(0..50), piece(50..100), piece(110..150)];
        // an onset 50 ms before the change moves the boundary there
        assert_eq!(
            snap_to_onsets(pieces(), &peaks(&[0.45], 0.5), 0.01),
            vec![piece(0..45), piece(45..100), piece(110..150)]
        );
        // the nearest of several; one frame after the change counts, two do not
        assert_eq!(
            snap_to_onsets(pieces(), &peaks(&[0.43, 0.47], 0.5), 0.01)[0].range,
            0..47
        );
        assert_eq!(
            snap_to_onsets(pieces(), &peaks(&[0.51], 0.5), 0.01)[0].range,
            0..51
        );
        assert_eq!(
            snap_to_onsets(pieces(), &peaks(&[0.52], 0.5), 0.01),
            pieces()
        );
        // at most PITCH_SNAP_MS before: 80 ms moves, 90 ms does not
        assert_eq!(
            snap_to_onsets(pieces(), &peaks(&[0.42], 0.5), 0.01)[0].range,
            0..42
        );
        assert_eq!(
            snap_to_onsets(pieces(), &peaks(&[0.41], 0.5), 0.01),
            pieces()
        );
        // a weak onset, a gap between pieces, or a piece left empty: no move
        assert_eq!(
            snap_to_onsets(pieces(), &peaks(&[0.45], ONSET_SPLIT_MIN - 0.01), 0.01),
            pieces()
        );
        assert_eq!(
            snap_to_onsets(pieces(), &peaks(&[1.05], 1.0), 0.01),
            pieces()
        );
        let short = vec![piece(40..50), piece(50..100)];
        assert_eq!(
            snap_to_onsets(short.clone(), &peaks(&[0.40, f64::NAN], 1.0), 0.01),
            short,
            "the earlier piece keeps a frame"
        );
        // through segment: a legato change seen late starts at the onset
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(62.0), 30)]), 0.9);
        let notes = segment(&t, &peaks(&[0.26], 0.6), 0.0);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!((notes[1].start - 0.26).abs() < 1e-6, "{notes:?}");
        assert!(
            !notes[1].split_by_onset,
            "a snapped change is not an onset split"
        );
    }

    #[test]
    fn glide_into_a_note_is_part_of_it() {
        let piece = |r: Range<usize>| Piece {
            range: r,
            onset: None,
        };
        let skip = ms_to_frames(TUNING_SKIP_MS, 0.01);
        let max = ms_to_frames(GLIDE_MAX_MS, 0.01);
        // 80 ms gliding from 58 up to 59.9, then 60 held
        let mut midi: Vec<Option<f32>> = (0..8).map(|k| Some(58.0 + 0.27 * k as f32)).collect();
        midi.extend(run(Some(60.0), 30));
        let t = track_of(&midi, 0.9);
        let f = analyse(&t, std::slice::from_ref(&(0..38)), 0.0);
        assert_eq!(
            merge_glides(vec![piece(0..8), piece(8..38)], &f, max, skip),
            vec![piece(0..38)]
        );
        // a glide away from the next note, or one ending short of it, stays
        let mut away: Vec<Option<f32>> = (0..8).map(|k| Some(60.5 - 0.3 * k as f32)).collect();
        away.extend(run(Some(62.0), 30));
        let f = analyse(&track_of(&away, 0.9), std::slice::from_ref(&(0..38)), 0.0);
        assert_eq!(
            merge_glides(vec![piece(0..8), piece(8..38)], &f, max, skip).len(),
            2
        );
        // a held note whose end a late change left in the piece is not a glide: its median
        // sits at its start
        let mut held: Vec<Option<f32>> = run(Some(62.0), 6);
        held.extend(run(Some(60.0), 32));
        let f = analyse(&track_of(&held, 0.9), std::slice::from_ref(&(0..38)), 0.0);
        assert_eq!(
            merge_glides(vec![piece(0..8), piece(8..38)], &f, max, skip).len(),
            2
        );
        // a piece of 100 ms or more is a note, however it moves
        let mut long: Vec<Option<f32>> = (0..10).map(|k| Some(58.0 + 0.2 * k as f32)).collect();
        long.extend(run(Some(60.0), 30));
        let f = analyse(&track_of(&long, 0.9), std::slice::from_ref(&(0..40)), 0.0);
        assert_eq!(
            merge_glides(vec![piece(0..10), piece(10..40)], &f, max, skip).len(),
            2
        );
        // pieces that are not adjacent do not merge
        let f = analyse(&t, std::slice::from_ref(&(0..38)), 0.0);
        assert_eq!(
            merge_glides(vec![piece(0..7), piece(8..38)], &f, max, skip).len(),
            2
        );
        // the merged note keeps a re-attack: an onset inside it still splits it afterwards
        let mut midi: Vec<Option<f32>> = (0..8).map(|k| Some(58.0 + 0.27 * k as f32)).collect();
        midi.extend(run(Some(60.0), 40));
        let t = track_of(&midi, 0.9);
        let notes = segment(&t, &peaks(&[0.25], 0.8), 0.0);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(
            notes.iter().all(|n| (n.midi - 60.0).abs() < 0.05),
            "{notes:?}"
        );
    }

    #[test]
    fn vibrato_portamento_and_glides_keep_every_note() {
        // the evaluation's hardest voice: vibrato, attack glides, portamento and repeats
        let settings = SynthSettings {
            vibrato_cents: 50.0,
            attack_glide_ms: 60.0,
            portamento_ms: 60.0,
            repeated_notes: true,
            ..voice()
        };
        for seed in [1005u64, 1007, 1008] {
            let mut case = Case::generate(seed, &settings);
            case.truth.truncate(40);
            let samples = case.render(ANALYSIS_RATE);
            let t = track(&samples);
            let notes = segment(&t, &onset_strength(&samples), estimate_tuning(&t));
            let reference: Vec<_> = case.truth.iter().map(Into::into).collect();
            let estimate: Vec<crate::eval::score::ScoredNote> = notes
                .iter()
                .map(|n| crate::eval::score::ScoredNote {
                    onset: n.start,
                    midi: f64::from(n.midi),
                })
                .collect();
            let score = crate::eval::score::conp(&reference, &estimate);
            assert!(score.f1 >= 0.97, "seed {seed}: {score:?}");
        }
    }

    #[test]
    fn median_pitch_skips_attack() {
        // a 30 ms attack well below the note does not move its pitch
        let t = track_of(&frames(&[(Some(57.0), 3), (Some(60.2), 30)]), 0.9);
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!((notes[0].midi - 60.2).abs() < 1e-3, "{notes:?}");
        // nor does a scoop longer than the attack skip: those frames are still in a glide
        let mut scoop: Vec<Option<f32>> = (0..6).map(|k| Some(59.0 + 0.2 * k as f32)).collect();
        scoop.extend(run(Some(60.2), 30));
        let notes = segment(&track_of(&scoop, 0.9), &no_onsets(), 0.0);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!((notes[0].midi - 60.2).abs() < 1e-3, "{notes:?}");
        // synth attack glides of 100 cents
        let settings = SynthSettings {
            attack_glide_ms: 60.0,
            ..voice()
        };
        let notes = transcribe(&audio(&[(0.2, 0.6, 62), (0.8, 1.1, 66)], settings));
        assert_eq!(notes.len(), 2);
        for (n, want) in notes.iter().zip([62.0, 66.0]) {
            assert!((n.midi - want).abs() <= 0.1, "{n:?}");
        }
    }

    #[test]
    fn drops_notes_under_60ms() {
        // a 50 ms blip alone is dropped, 60 ms is kept
        let t = track_of(
            &frames(&[
                (None, 10),
                (Some(60.0), 5),
                (None, 10),
                (Some(64.0), 6),
                (None, 5),
            ]),
            0.9,
        );
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!((notes[0].midi - 64.0).abs() < 1e-3);
        // a short note far from its neighbours' pitches is dropped
        let t = track_of(
            &frames(&[
                (Some(60.0), 30),
                (None, 5),
                (Some(67.0), 5),
                (None, 5),
                (Some(60.0), 30),
            ]),
            0.9,
        );
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes.iter().all(|n| (n.midi - 60.0).abs() < 1e-3));
    }

    #[test]
    fn merges_short_note_into_neighbour() {
        let t = track_of(
            &frames(&[
                (Some(60.0), 30),
                (Some(60.4), 4),
                (Some(60.9), 30),
                (Some(64.0), 4),
            ]),
            0.9,
        );
        let f = analyse(&t, std::slice::from_ref(&(0..68)), 0.0);
        let piece = |r: Range<usize>| Piece {
            range: r,
            onset: None,
        };
        let skip = ms_to_frames(TUNING_SKIP_MS, 0.01);
        let min = ms_to_frames(MIN_NOTE_MS, 0.01);
        // 60.4 is within the change threshold of both: the closer (60.0 by 0.4, 60.9 by 0.5) wins
        let merged = merge_short(
            vec![piece(0..30), piece(30..34), piece(34..64), piece(64..68)],
            &f,
            min,
            skip,
        );
        // the trailing 64 has no neighbour within reach and is dropped
        assert_eq!(
            merged,
            vec![piece(0..34), piece(34..64)],
            "into the closer neighbour"
        );
        // a short note only next to the later neighbour merges into it and keeps its own start
        let merged = merge_short(vec![piece(30..34), piece(34..64)], &f, min, skip);
        assert_eq!(merged, vec![piece(30..64)]);
        // pieces that are not adjacent do not merge
        let merged = merge_short(vec![piece(0..30), piece(31..34)], &f, min, skip);
        assert_eq!(merged, vec![piece(0..30)]);
    }

    #[test]
    fn rejoins_a_split_with_the_same_pitch() {
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(60.3), 30)]), 0.9);
        let f = analyse(&t, std::slice::from_ref(&(0..60)), 0.0);
        let piece = |r: Range<usize>| Piece {
            range: r,
            onset: None,
        };
        assert_eq!(
            merge_same_pitch(vec![piece(0..30), piece(30..60)], &f, 3),
            vec![piece(0..60)]
        );
        let t = track_of(&frames(&[(Some(60.0), 30), (Some(61.0), 30)]), 0.9);
        let f = analyse(&t, std::slice::from_ref(&(0..60)), 0.0);
        assert_eq!(
            merge_same_pitch(vec![piece(0..30), piece(30..60)], &f, 3),
            vec![piece(0..30), piece(30..60)]
        );
    }

    #[test]
    fn confidence_components_in_range() {
        let settings = [
            SynthSettings::default(),
            SynthSettings {
                vibrato_cents: 50.0,
                attack_glide_ms: 60.0,
                portamento_ms: 40.0,
                repeated_notes: true,
                breath_noise_db: Some(-30.0),
                noise_floor_db: Some(-45.0),
                ..SynthSettings::default()
            },
        ];
        for (k, s) in settings.iter().enumerate() {
            let case = Case::generate(1000 + k as u64, s);
            let notes = transcribe(&case.render(ANALYSIS_RATE));
            assert!(!notes.is_empty());
            for n in &notes {
                let c = &n.components;
                for (name, v) in [
                    ("voiced", c.voiced),
                    ("stability", c.stability),
                    ("fit", c.fit),
                    ("confidence", n.confidence),
                ] {
                    assert!((0.0..=1.0).contains(&v), "case {k}: {name} {v} in {n:?}");
                }
                let mut want = c.voiced.powf(CONF_W_VOICED)
                    * c.stability.powf(CONF_W_STABILITY)
                    * c.fit.powf(CONF_W_FIT);
                if let Some(o) = c.onset {
                    assert!((0.0..=1.0).contains(&o) && n.split_by_onset);
                    want *= 0.5 + 0.5 * o;
                } else {
                    assert!(!n.split_by_onset);
                }
                assert!((n.confidence - want).abs() < 1e-6, "case {k}: {n:?}");
            }
        }
        // a clean steady tone is confident in every component
        let notes = transcribe(&audio(&[(0.2, 1.2, 69)], voice()));
        assert_eq!(notes.len(), 1);
        let c = &notes[0].components;
        assert!(c.voiced > 0.8 && c.stability > 0.9 && c.fit > 0.9, "{c:?}");
    }

    #[test]
    fn a_piece_needs_a_pitched_frame_with_credible_voicing() {
        let mut t = track_of(
            &frames(&[(Some(60.0), 5), (None, 2), (Some(60.0), 5)]),
            MIN_VOICED_PROB_EMIT - 0.01,
        );
        for frame in &mut t.frames {
            frame.rms = 0.001;
        }
        // High voicing probability in a gap cannot support either pitched side.
        t.frames[5].voiced_prob = 0.9;
        assert!(segment(&t, &no_onsets(), 0.0).is_empty());

        // A quiet note with just one supported frame keeps both its attack and bridged gap.
        t.frames[9].voiced_prob = MIN_VOICED_PROB_EMIT;
        let notes = segment(&t, &no_onsets(), 0.0);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].start, 0.0);
        assert!((notes[0].end - 0.12).abs() < 1e-6);
        assert_eq!(notes[0].components.voiced, MIN_VOICED_PROB_EMIT / 12.0);
    }

    #[test]
    fn confidence_drops_for_off_pitch_note() {
        let note = [(0.2, 1.0, 69)];
        let in_tune = transcribe(&audio(&note, voice()));
        let off = audio(
            &note,
            SynthSettings {
                detune_cents: 40.0,
                ..voice()
            },
        );
        // tuning off, so the 40 cents stay in the note
        let off = segment(&track(&off), &onset_strength(&off), 0.0);
        assert_eq!((in_tune.len(), off.len()), (1, 1));
        let (a, b) = (&in_tune[0], &off[0]);
        assert!(
            (b.components.fit - 0.2).abs() <= 0.05,
            "40 cents off fits 0.2: {b:?}"
        );
        assert!(a.components.fit > 0.95, "{a:?}");
        assert!(b.confidence < a.confidence - 0.2, "{a:?} vs {b:?}");
    }

    #[test]
    fn empty_track_gives_no_notes() {
        assert!(segment(&track_of(&[], 0.9), &no_onsets(), 0.0).is_empty());
        assert!(segment(&track_of(&run(None, 100), 0.0), &no_onsets(), 0.0).is_empty());
        assert!(transcribe(&[]).is_empty());
        assert!(transcribe(&vec![0.0; 16_000]).is_empty(), "silence");
        let mut t = track_of(&run(Some(60.0), 50), 0.9);
        t.hop_seconds = f32::NAN;
        assert!(segment(&t, &no_onsets(), 0.0).is_empty(), "NaN hop");
        // a non-finite tuning is ignored, onsets past the track are ignored
        let t = track_of(&run(Some(60.0), 50), 0.9);
        let far = OnsetTrack {
            hop_seconds: hop(),
            strength: Vec::new(),
            rms: Vec::new(),
            peaks: vec![
                crate::onset::Onset {
                    seconds: 99.0,
                    strength: 1.0,
                },
                crate::onset::Onset {
                    seconds: -1.0,
                    strength: 1.0,
                },
            ],
        };
        let notes = segment(&t, &far, f32::NAN);
        assert_eq!(notes.len(), 1);
        assert!((notes[0].midi - 60.0).abs() < 1e-3);
    }

    #[test]
    fn notes_are_ordered_and_non_overlapping() {
        let settings = [
            SynthSettings::default(),
            SynthSettings {
                vibrato_cents: 50.0,
                portamento_ms: 40.0,
                repeated_notes: true,
                onset_jitter_ms: 20.0,
                ..SynthSettings::default()
            },
        ];
        for (k, s) in settings.iter().enumerate() {
            let case = Case::generate(1002 + k as u64, s);
            let notes = transcribe(&case.render(ANALYSIS_RATE));
            assert!(notes.len() >= case.truth.len() / 2, "case {k}");
            for n in &notes {
                assert!(n.start < n.end, "case {k}: {n:?}");
                assert!(n.end - n.start >= 0.06 - 1e-6, "case {k}: {n:?}");
            }
            for pair in notes.windows(2) {
                assert!(pair[0].end <= pair[1].start, "case {k}: {pair:?}");
            }
        }
    }

    #[test]
    fn tuning_is_subtracted_from_midi() {
        let t = track_of(&run(Some(69.0), 40), 0.9);
        for (cents, want) in [(30.0, 68.7), (-30.0, 69.3), (0.0, 69.0)] {
            let notes = segment(&t, &no_onsets(), cents);
            assert_eq!(notes.len(), 1);
            assert!(
                (notes[0].midi - want).abs() < 1e-3,
                "tuning {cents}: {notes:?}"
            );
            let fit = 1.0 - (want - want.round()).abs() * 2.0;
            assert!((notes[0].components.fit - fit).abs() < 1e-3);
        }
        // a voice 30 cents sharp, its tuning estimated, comes out on the semitone
        let samples = audio(
            &[(0.2, 0.7, 69), (0.7, 1.2, 72)],
            SynthSettings {
                detune_cents: 30.0,
                ..voice()
            },
        );
        let notes = transcribe(&samples);
        assert_eq!(notes.len(), 2);
        for (n, want) in notes.iter().zip([69.0, 72.0]) {
            assert!((n.midi - want).abs() <= 0.05, "{n:?}");
        }
    }

    #[test]
    fn segments_a_generated_melody() {
        let case = Case::generate(1000, &SynthSettings::default());
        let notes = transcribe(&case.render(ANALYSIS_RATE));
        let truth: Vec<(f64, f64, i32)> = case
            .truth
            .iter()
            .map(|n| (n.start, n.end, n.midi))
            .collect();
        assert_matches("seed 1000", &notes, &truth, 0.03);
    }
}
