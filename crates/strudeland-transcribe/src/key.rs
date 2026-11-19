//! The key: detected from the notes, or given as `--key ROOT:SCALE`.
//!
//! Detection is Krumhansl–Schmuckler: the pitch-class histogram of the notes, each weighted by
//! its raw duration times its confidence, is correlated (Pearson) with the Krumhansl–Kessler
//! major and minor profiles rotated to each of the 12 tonics. Two metrical cues weigh more, as in
//! key finders that use metre (Temperley 2001, ch. 7): a note attacked on a downbeat (step 0 of
//! bar 1 or later) adds `KEY_DOWNBEAT_BONUS` mean notes of weight, and the final note
//! `KEY_FINAL_BONUS`, since melodies settle on their tonic. Without them a melody's dominant or relative key often correlates as well as its
//! own. The best of the 24 keys wins, its
//! confidence is its margin over the runner-up. With fewer than `KEY_MIN_NOTES` notes (parts of
//! a note split at a barline count once) or a margin below `KEY_MARGIN_MIN` no key is chosen.
//!
//! A given key may use any of the 12 scales of [`SCALE_STEPS`]: scale degrees need the scale's
//! interval set, so the set is the validation.

use serde::{Deserialize, Serialize};

use crate::consts::{KEY_DOWNBEAT_BONUS, KEY_FINAL_BONUS, KEY_MARGIN_MIN, KEY_MIN_NOTES};
use crate::error::TranscribeError;
use crate::report::Note;

/// The scales a key may use, with their semitones above the root (Tonal's interval sets).
pub const SCALE_STEPS: [(&str, &[u8]); 12] = [
    ("major", &[0, 2, 4, 5, 7, 9, 11]),
    ("minor", &[0, 2, 3, 5, 7, 8, 10]),
    ("dorian", &[0, 2, 3, 5, 7, 9, 10]),
    ("phrygian", &[0, 1, 3, 5, 7, 8, 10]),
    ("lydian", &[0, 2, 4, 6, 7, 9, 11]),
    ("mixolydian", &[0, 2, 4, 5, 7, 9, 10]),
    ("locrian", &[0, 1, 3, 5, 6, 8, 10]),
    ("harmonic minor", &[0, 2, 3, 5, 7, 8, 11]),
    ("melodic minor", &[0, 2, 3, 5, 7, 9, 11]),
    ("major pentatonic", &[0, 2, 4, 7, 9]),
    ("minor pentatonic", &[0, 3, 5, 7, 10]),
    ("blues", &[0, 3, 5, 6, 7, 10]),
];

/// The Krumhansl–Kessler major profile, tonic first (Krumhansl 1990).
const MAJOR_PROFILE: [f64; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
/// The Krumhansl–Kessler minor profile, tonic first (Krumhansl 1990).
const MINOR_PROFILE: [f64; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];

/// The uppercase sharp names of the pitch classes, C first: detected keys' roots.
const ROOTS: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// The uppercase sharp name of pitch class `pc` (taken modulo 12), as detected keys spell roots.
#[cfg(any(test, feature = "eval"))]
pub fn root_name(pc: u8) -> &'static str {
    ROOTS[usize::from(pc % 12)]
}

/// The reason no key is chosen for too few notes.
const TOO_FEW_NOTES: &str = "fewer than 6 notes";
/// The reason no key is chosen when the best two keys are too close.
const TOO_CLOSE: &str = "the best two keys are too close";

/// Why detection chose no key; displayed as the reason (`"fewer than 6 notes"`, `"the best two
/// keys are too close"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoKey {
    /// Fewer than `KEY_MIN_NOTES` notes.
    TooFewNotes,
    /// The margin over the runner-up is below `KEY_MARGIN_MIN`.
    TooClose,
}

impl std::fmt::Display for NoKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooFewNotes => TOO_FEW_NOTES,
            Self::TooClose => TOO_CLOSE,
        })
    }
}

/// The semitones above the root of a scale of [`SCALE_STEPS`].
pub fn scale_steps(name: &str) -> Option<&'static [u8]> {
    SCALE_STEPS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, steps)| *steps)
}

/// Where the key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KeySource {
    Detected,
    Flag,
}

/// The key of the transcription.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Key {
    /// `"A minor"`, `"C# major"`, `"Bb dorian"`: the root, a space, the scale.
    pub name: String,
    /// The root's pitch class, 0 is C.
    pub tonic_pc: u8,
    /// The Strudel scale name.
    pub scale: String,
    /// The correlation margin over the runner-up; null for a given key.
    pub confidence: Option<f32>,
    /// The second-best key's name; null for a given key.
    pub runner_up: Option<String>,
    /// The best key's correlation; null for a given key.
    pub correlation: Option<f32>,
    pub source: KeySource,
}

impl Key {
    /// The root as written in `name` (`"A"`, `"C#"`, `"Bb"`).
    pub fn root(&self) -> &str {
        self.name.split(' ').next().unwrap_or("")
    }

    /// The scale's semitones above the root; `None` for a scale outside the 12 a key may use.
    pub fn steps(&self) -> Option<&'static [u8]> {
        scale_steps(&self.scale)
    }
}

/// The pitch class of a root such as `C`, `c#` or `Bb`; `None` when it is not a letter A-G
/// (either case) with an optional `#` or `b`.
fn root_pc(root: &str) -> Option<u8> {
    let mut chars = root.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let base: i32 = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let offset = match chars.next() {
        None => 0,
        Some('#') => 1,
        Some('b') => -1,
        Some(_) => return None,
    };
    if chars.next().is_some() {
        return None;
    }
    Some((base + offset).rem_euclid(12) as u8)
}

/// Parses `--key ROOT:SCALE`: a root letter A-G (either case) with an optional `#` or `b`, and a
/// scale of `SCALE_STEPS` (either case), as the key it names: the root normalized to an
/// uppercase letter, `source: Flag` and no confidence, runner-up or correlation.
pub fn parse_key_override(s: &str) -> Result<Key, TranscribeError> {
    let bad =
        || TranscribeError::BadKey(format!("bad --key \"{s}\"; use ROOT:SCALE such as C:major"));
    let (root, scale) = s.split_once(':').ok_or_else(bad)?;
    let tonic_pc = root_pc(root).ok_or_else(bad)?;
    if scale.is_empty() {
        return Err(bad());
    }
    let lower = scale.to_lowercase();
    let Some((name, _)) = SCALE_STEPS.iter().find(|(n, _)| *n == lower) else {
        let names: Vec<&str> = SCALE_STEPS.iter().map(|(n, _)| *n).collect();
        return Err(TranscribeError::BadKey(format!(
            "unknown scale \"{scale}\"; use one of: {}",
            names.join(", ")
        )));
    };
    let mut chars = root.chars();
    let mut root_text = String::new();
    if let Some(letter) = chars.next() {
        root_text.push(letter.to_ascii_uppercase());
    }
    root_text.extend(chars);
    Ok(Key {
        name: format!("{root_text} {name}"),
        tonic_pc,
        scale: (*name).to_string(),
        confidence: None,
        runner_up: None,
        correlation: None,
        source: KeySource::Flag,
    })
}

/// Pearson correlation; 0.0 when either side is constant.
fn pearson(x: &[f64; 12], y: &[f64; 12]) -> f64 {
    let mx = x.iter().sum::<f64>() / 12.0;
    let my = y.iter().sum::<f64>() / 12.0;
    let (mut num, mut sx, mut sy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        num += (a - mx) * (b - my);
        sx += (a - mx).powi(2);
        sy += (b - my).powi(2);
    }
    let den = (sx * sy).sqrt();
    if den > 0.0 && den.is_finite() {
        num / den
    } else {
        0.0
    }
}

/// The weighted pitch-class histogram of `notes`: each note counts its duration times its
/// confidence (a note of no weight, not finite or not positive, counts nothing). On top of that,
/// with `mean` the mean weight of the notes that count, a note attacked on a downbeat (step 0 of
/// bar 1 or later, not the continuation of a note split at the barline) adds
/// `KEY_DOWNBEAT_BONUS * mean`, and the final note (the last one, with any parts it continues)
/// adds `KEY_FINAL_BONUS * mean` once. The bonuses are in mean notes, so they weigh the same in
/// a short melody and a long one.
fn key_histogram(notes: &[Note]) -> [f64; 12] {
    let weight = |n: &Note| {
        let w = (n.end - n.start) * f64::from(n.confidence);
        if w.is_finite() && w > 0.0 { w } else { 0.0 }
    };
    let weights: Vec<f64> = notes.iter().map(weight).collect();
    let counted = weights.iter().filter(|w| **w > 0.0).count();
    let mut histogram = [0.0f64; 12];
    if counted == 0 {
        return histogram;
    }
    let mean = weights.iter().sum::<f64>() / counted as f64;
    for (i, (n, w)) in notes.iter().zip(&weights).enumerate() {
        if *w <= 0.0 {
            continue;
        }
        let continuation = i > 0 && notes[i - 1].tied_to_next;
        let downbeat = n.bar.is_some_and(|b| b >= 1) && n.step == Some(0) && !continuation;
        let bonus = if downbeat {
            KEY_DOWNBEAT_BONUS * mean
        } else {
            0.0
        };
        histogram[n.midi.rem_euclid(12) as usize] += w + bonus;
    }
    // the final note: the last note of weight and the parts it continues
    if let Some(last) = weights.iter().rposition(|w| *w > 0.0) {
        histogram[notes[last].midi.rem_euclid(12) as usize] += KEY_FINAL_BONUS * mean;
    }
    histogram
}

/// The key of `notes` by Krumhansl–Schmuckler over the 24 major and minor keys, or why none is
/// chosen.
pub fn detect_key(notes: &[Note]) -> Result<Key, NoKey> {
    // a note split at a barline is one note: its continuations follow a `tied_to_next` part
    let count = notes
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || !notes[i - 1].tied_to_next)
        .count();
    if count < KEY_MIN_NOTES {
        return Err(NoKey::TooFewNotes);
    }
    let histogram = key_histogram(notes);
    // (correlation, tonic, minor), best first; ties keep major before minor and the lower tonic
    let mut scored: Vec<(f64, u8, bool)> = Vec::with_capacity(24);
    for minor in [false, true] {
        let profile = if minor {
            &MINOR_PROFILE
        } else {
            &MAJOR_PROFILE
        };
        for tonic in 0..12u8 {
            let mut rotated = [0.0f64; 12];
            for (pc, slot) in rotated.iter_mut().enumerate() {
                *slot = profile[(pc + 12 - usize::from(tonic)) % 12];
            }
            scored.push((pearson(&histogram, &rotated), tonic, minor));
        }
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (best, runner) = (scored[0], scored[1]);
    let margin = best.0 - runner.0;
    if margin.is_nan() || margin < f64::from(KEY_MARGIN_MIN) {
        return Err(NoKey::TooClose);
    }
    let name = |(_, tonic, minor): (f64, u8, bool)| {
        format!(
            "{} {}",
            ROOTS[usize::from(tonic)],
            if minor { "minor" } else { "major" }
        )
    };
    Ok(Key {
        name: name(best),
        tonic_pc: best.1,
        scale: if best.2 { "minor" } else { "major" }.to_string(),
        confidence: Some(margin as f32),
        runner_up: Some(name(runner)),
        correlation: Some(best.0 as f32),
        source: KeySource::Detected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Components;

    /// A note of `midi` lasting `seconds` at `confidence`, starting at `at`.
    fn note(at: f64, seconds: f64, midi: i32, confidence: f32) -> Note {
        Note {
            start: at,
            end: at + seconds,
            bar: Some(1),
            step: Some(0),
            steps: Some(1),
            midi,
            name: crate::notation::midi_to_name(midi),
            degree: None,
            cents: 0.0,
            confidence,
            components: Components {
                voiced: 1.0,
                stability: 1.0,
                fit: 1.0,
                onset: None,
            },
            snap_error_ms: None,
            tied_to_next: false,
            alternative: None,
            early: false,
        }
    }

    /// One note per `(midi, seconds)`, back to back, confidence 1.
    fn melody(notes: &[(i32, f64)]) -> Vec<Note> {
        let mut at = 0.0;
        notes
            .iter()
            .map(|&(midi, seconds)| {
                let n = note(at, seconds, midi, 1.0);
                at += seconds;
                n
            })
            .collect()
    }

    #[test]
    fn detects_c_major_scale() {
        // C D E F G A B C, the tonic and the fifth held longer
        let notes = melody(&[
            (60, 1.0),
            (62, 0.5),
            (64, 0.5),
            (65, 0.5),
            (67, 1.0),
            (69, 0.5),
            (71, 0.5),
            (72, 1.0),
        ]);
        let key = detect_key(&notes).unwrap();
        assert_eq!(key.name, "C major");
        assert_eq!(key.tonic_pc, 0);
        assert_eq!(key.scale, "major");
        assert_eq!(key.source, KeySource::Detected);
        let margin = key.confidence.unwrap();
        assert!(margin >= KEY_MARGIN_MIN, "margin {margin}");
        assert!(key.correlation.unwrap() > 0.8, "{key:?}");
        assert!(key.runner_up.is_some());
    }

    #[test]
    fn detects_a_minor_scale() {
        // A B C D E F G A, the tonic and the fifth held longer
        let notes = melody(&[
            (57, 1.0),
            (59, 0.5),
            (60, 0.5),
            (62, 0.5),
            (64, 1.0),
            (65, 0.5),
            (67, 0.5),
            (69, 1.0),
        ]);
        let key = detect_key(&notes).unwrap();
        assert_eq!(key.name, "A minor");
        assert_eq!(key.tonic_pc, 9);
        assert_eq!(key.scale, "minor");
        assert!(key.confidence.unwrap() >= KEY_MARGIN_MIN, "{key:?}");
    }

    #[test]
    fn weights_by_duration_and_confidence() {
        // the same pitches; which of C and G is long decides between C major and G major (the
        // notes are off the beat and end on D, in both keys, and the set is played four times
        // with one long note, so the final note's bonus stays small)
        let pitches = [60, 62, 65, 66, 67, 69, 71, 62];
        let weighted = |long: i32| {
            let mut notes = melody(
                &(0..4)
                    .flat_map(|round| {
                        pitches
                            .iter()
                            .map(move |&m| (m, if m == long && round == 0 { 4.0 } else { 0.25 }))
                    })
                    .collect::<Vec<_>>(),
            );
            for n in &mut notes {
                n.step = Some(2);
            }
            notes
        };
        assert_eq!(detect_key(&weighted(60)).unwrap().name, "C major");
        assert_eq!(detect_key(&weighted(67)).unwrap().name, "G major");
        // a long note at confidence 0 counts nothing: the G-heavy melody with its long G at
        // confidence 0 and a long C at full confidence is C major again
        let mut notes = weighted(67);
        let g = notes.iter_mut().find(|n| n.midi == 67).unwrap();
        g.confidence = 0.0;
        let c = notes.iter_mut().find(|n| n.midi == 60).unwrap();
        c.end = c.start + 4.0;
        assert_eq!(detect_key(&notes).unwrap().name, "C major");
    }

    /// `pitches` as equal off-beat notes (step 2 of bar 1), then `last` as the final note.
    fn off_beat(pitches: &[i32], last: i32) -> Vec<Note> {
        let mut notes: Vec<(i32, f64)> = pitches.iter().map(|&m| (m, 0.5)).collect();
        notes.push((last, 0.5));
        let mut notes = melody(&notes);
        for n in &mut notes {
            n.step = Some(2);
        }
        notes
    }

    #[test]
    fn weights_downbeats_and_the_final_note() {
        // C D E G A B twice: a C major and G major pitch set without F or F#
        let set = [60, 62, 64, 67, 69, 71, 72, 74, 76, 79, 81, 83];
        // the final note decides
        assert_eq!(detect_key(&off_beat(&set, 72)).unwrap().name, "C major");
        assert_eq!(detect_key(&off_beat(&set, 67)).unwrap().name, "G major");
    }

    #[test]
    fn key_histogram_weights() {
        // off-beat notes count duration times confidence; the final note adds its bonus in
        // mean notes (the mean here is (0.5 + 0.25 + 0.5) / 3)
        let mut notes = off_beat(&[60, 62], 64);
        notes[1].confidence = 0.5;
        let mean = 1.25 / 3.0;
        let h = key_histogram(&notes);
        assert_eq!((h[0], h[2]), (0.5, 0.25));
        assert_eq!(h[4], 0.5 + KEY_FINAL_BONUS * mean);
        // downbeats in bar 1 or later add theirs; the final note on a downbeat takes both
        let mut notes = off_beat(&[60, 62], 64);
        notes[0].step = Some(0);
        notes[2].step = Some(0);
        notes[2].bar = Some(3);
        let h = key_histogram(&notes);
        assert_eq!(h[0], 0.5 + KEY_DOWNBEAT_BONUS * 0.5);
        assert_eq!(h[4], 0.5 + KEY_DOWNBEAT_BONUS * 0.5 + KEY_FINAL_BONUS * 0.5);
        // step 0 of bar 0 (before the origin), of no bar (unresolved), or a barline
        // continuation is no downbeat
        for (bar, tied) in [(Some(0), false), (None, false), (Some(2), true)] {
            let mut notes = off_beat(&[60, 62, 65], 64);
            notes[1].step = Some(0);
            notes[1].bar = bar;
            notes[0].tied_to_next = tied;
            assert_eq!(key_histogram(&notes)[2], 0.5, "bar {bar:?}, tied {tied}");
        }
        // a final note split at a barline: the bonus once, on its pitch class
        let mut notes = off_beat(&[60, 62, 67], 67);
        notes[2].tied_to_next = true;
        notes[3].step = Some(0);
        let h = key_histogram(&notes);
        assert_eq!(h[7], 1.0 + KEY_FINAL_BONUS * 0.5, "{h:?}");
        // the final note is the last one of weight
        let mut notes = off_beat(&[60, 62], 64);
        notes[2].confidence = 0.0;
        let h = key_histogram(&notes);
        assert_eq!((h[2], h[4]), (0.5 + KEY_FINAL_BONUS * 0.5, 0.0));
        // no weight: not finite, zero length, zero confidence; and no notes at all
        let mut notes = off_beat(&[60, 62], 64);
        notes[0].end = notes[0].start;
        notes[1].confidence = f32::NAN;
        notes[2].end = f64::INFINITY;
        assert_eq!(key_histogram(&notes), [0.0; 12]);
        assert_eq!(key_histogram(&[]), [0.0; 12]);
    }

    #[test]
    fn fewer_than_6_notes_no_key() {
        let notes = melody(&[(60, 1.0), (62, 1.0), (64, 1.0), (65, 1.0), (67, 1.0)]);
        assert_eq!(detect_key(&notes).unwrap_err(), NoKey::TooFewNotes);
        assert_eq!(detect_key(&[]).unwrap_err(), NoKey::TooFewNotes);
        // six parts, but two are one note split at a barline
        let mut notes = melody(&[
            (60, 1.0),
            (62, 1.0),
            (64, 1.0),
            (65, 1.0),
            (67, 1.0),
            (67, 1.0),
        ]);
        notes[4].tied_to_next = true;
        assert_eq!(detect_key(&notes).unwrap_err(), NoKey::TooFewNotes);
    }

    #[test]
    fn small_margin_no_key() {
        // all 12 pitch classes equally: every key correlates the same
        let chromatic: Vec<(i32, f64)> = (60..72).map(|m| (m, 1.0)).collect();
        assert_eq!(
            detect_key(&melody(&chromatic)).unwrap_err(),
            NoKey::TooClose
        );
        // a whole-tone set is ambiguous between its transpositions
        let whole_tone = melody(&[
            (60, 1.0),
            (62, 1.0),
            (64, 1.0),
            (66, 1.0),
            (68, 1.0),
            (70, 1.0),
        ]);
        assert_eq!(detect_key(&whole_tone).unwrap_err(), NoKey::TooClose);
        // no weight at all (every confidence 0)
        let mut silent = melody(&chromatic);
        for n in &mut silent {
            n.confidence = 0.0;
        }
        assert_eq!(detect_key(&silent).unwrap_err(), NoKey::TooClose);
        assert_eq!(NoKey::TooFewNotes.to_string(), "fewer than 6 notes");
        assert_eq!(
            NoKey::TooClose.to_string(),
            "the best two keys are too close"
        );
    }

    #[test]
    fn flag_sets_source_and_nulls_confidence() {
        let key = parse_key_override("bb:Dorian").unwrap();
        assert_eq!(
            key,
            Key {
                name: "Bb dorian".into(),
                tonic_pc: 10,
                scale: "dorian".into(),
                confidence: None,
                runner_up: None,
                correlation: None,
                source: KeySource::Flag,
            }
        );
        assert_eq!(key.root(), "Bb");
        let json = serde_json::to_value(&key).unwrap();
        assert_eq!(json["source"], "flag");
        for field in ["confidence", "runnerUp", "correlation"] {
            assert!(json[field].is_null(), "{field} is present and null");
        }
    }

    #[test]
    fn parse_key_override_cases() {
        let ok = |s: &str, root: &str, scale: &str| {
            let key = parse_key_override(s).unwrap();
            assert_eq!((key.root(), key.scale.as_str()), (root, scale), "{s}");
            assert_eq!(key.name, format!("{root} {scale}"), "{s}");
        };
        ok("C:major", "C", "major");
        ok("c#:minor", "C#", "minor");
        ok("Bb:dorian", "Bb", "dorian");
        ok("A:harmonic minor", "A", "harmonic minor");
        ok("g:MIXOLYDIAN", "G", "mixolydian");
        assert_eq!(parse_key_override("Cb:major").unwrap().tonic_pc, 11);
        assert_eq!(parse_key_override("B#:major").unwrap().tonic_pc, 0);
        for bad in [
            "H:major",
            "C##:major",
            "Cx:major",
            "C major",
            "",
            ":major",
            "C:",
            "C:major:x",
            "#:major",
        ] {
            let err = parse_key_override(bad).unwrap_err();
            if bad == "C:major:x" {
                // the scale is "major:x"
                assert!(err.to_string().starts_with("unknown scale"), "{bad}: {err}");
                continue;
            }
            assert_eq!(
                err.to_string(),
                format!("bad --key \"{bad}\"; use ROOT:SCALE such as C:major"),
                "{bad}"
            );
            assert_eq!(err.exit_code(), 4, "{bad}");
            assert!(matches!(err, TranscribeError::BadKey(_)), "{bad}");
        }
    }

    #[test]
    fn unknown_scale_lists_choices() {
        let err = parse_key_override("C:bebop").unwrap_err();
        assert!(matches!(err, TranscribeError::BadKey(_)), "{err:?}");
        assert_eq!(
            err.to_string(),
            "unknown scale \"bebop\"; use one of: major, minor, dorian, phrygian, lydian, \
             mixolydian, locrian, harmonic minor, melodic minor, major pentatonic, minor \
             pentatonic, blues"
        );
    }

    #[test]
    fn scale_steps_are_ascending_from_the_root() {
        assert_eq!(SCALE_STEPS.len(), 12);
        for (name, steps) in SCALE_STEPS {
            assert_eq!(steps[0], 0, "{name}");
            assert!(steps.windows(2).all(|w| w[0] < w[1]), "{name}");
            assert!(*steps.last().unwrap() < 12, "{name}");
            assert_eq!(scale_steps(name), Some(steps), "{name}");
        }
        assert_eq!(scale_steps("bebop"), None);
    }
}
