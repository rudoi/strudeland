//! Strudel mini-notation for quantized notes: note names, and scale degrees when there is a key.
//!
//! One cycle is one bar. Within a bar the notes and the gaps between them become events with
//! `@n` lengths and `~` rests, every length divided by the GCD of the bar's lengths (so
//! `c4 ~ e4@2`, not sixteen tokens); a bar without notes is `~`. One bar is a flat sequence,
//! several bars `<[bar 1] [bar 2] ...>`. The pattern starts at its `first_bar` (the bar the
//! analysed span starts in, or 0, the pickup bar before bar 1, when the span starts before bar 1
//! and has notes there), not always at bar 1. Notes without a step (an unresolved grid) are left
//! out. The scale root comes from the notes from bar 1 on, so a pickup does not move it. A note split at a barline is two events, the second
//! re-attacked: the mini-notation has no tie across cycles.
//!
//! Scale degrees count from the key's root at the octave at or below the median of the rendered
//! notes (the root's octave contains the median, so most degrees are positive), written
//! `.scale("A3:minor")`. Strudel maps degree 0 to that root and `d` to octave `floor(d / len)`
//! and scale step `d mod len` (`vendor/strudel/packages/tonal/tonal.mjs`). A note outside the
//! scale is the scale degree below it plus one `#` per semitone above that degree; negative
//! degrees are notes below the root.

use crate::key::Key;
use crate::report::{Notation, Note};

/// The lowercase sharp names of the pitch classes, C first.
const PITCH_CLASSES: [&str; 12] = [
    "c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b",
];

/// A MIDI note as a lowercase mini-notation name with sharps: 60 is `c4`, 61 `c#4`, 0 `c-1`.
pub fn midi_to_name(midi: i32) -> String {
    let pc = midi.rem_euclid(12) as usize;
    let octave = i64::from(midi.div_euclid(12)) - 1;
    format!("{}{octave}", PITCH_CLASSES[pc])
}

/// The scale degree of `midi` counted from the scale root at `root_midi` (a MIDI note of the
/// key's tonic): `"0"` for the root, `"-2"` below it, `"1#"` for a note a semitone above degree
/// 1 and outside the scale. A key whose scale has no interval set counts as chromatic.
pub fn scale_degree(midi: i32, root_midi: i32, key: &Key) -> String {
    let steps: &[u8] = key
        .steps()
        .unwrap_or(&[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    let len = steps.len() as i64;
    let rel = i64::from(midi) - i64::from(root_midi);
    let octave = rel.div_euclid(12);
    let pc = rel.rem_euclid(12);
    // the highest scale step at or below the pitch class (step 0 is 0, so one always exists)
    let (index, step) = steps
        .iter()
        .enumerate()
        .rev()
        .find(|(_, s)| i64::from(**s) <= pc)
        .map_or((0, 0), |(i, s)| (i as i64, i64::from(*s)));
    let degree = octave * len + index;
    let sharps = (pc - step).max(0) as usize;
    format!("{degree}{}", "#".repeat(sharps))
}

/// The notes the notation renders: a bar of 1 or more, a step and a length.
fn rendered(notes: &[Note]) -> impl Iterator<Item = &Note> {
    notes
        .iter()
        .filter(|n| n.bar.is_some_and(|b| b >= 1) && n.step.is_some() && n.steps.is_some())
}

/// The scale root as a MIDI note: the key's tonic at or below the median of `notes` (the lower
/// middle value for an even count), C4's octave when `notes` is empty.
pub fn scale_root_midi(notes: &[&Note], key: &Key) -> i32 {
    let mut midis: Vec<i64> = notes.iter().map(|n| i64::from(n.midi)).collect();
    midis.sort_unstable();
    let median = midis
        .get(midis.len().saturating_sub(1) / 2)
        .copied()
        .unwrap_or(60);
    let tonic = i64::from(key.tonic_pc % 12);
    let root = median - (median - tonic).rem_euclid(12);
    root.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// The `.scale(...)` argument: the key's root, the root's octave, a colon and the scale. The
/// octave is the one of the root's letter, as tonal reads it (`Note.midi`): `Cb4` is MIDI 59 and
/// `B#3` MIDI 60, so a root spelled across the B/C boundary is not numbered from its MIDI note.
fn scale_label(key: &Key, root_midi: i32) -> String {
    let root = key.root();
    let accidental: i64 = root
        .chars()
        .skip(1)
        .map(|c| match c {
            '#' => 1,
            'b' => -1,
            _ => 0,
        })
        .sum();
    let letter_midi = i64::from(root_midi) - accidental;
    let octave = letter_midi.div_euclid(12) - 1;
    format!("{root}{octave}:{}", key.scale)
}

/// Fills every note's `degree` from `key` (or clears it without a key). The root octave comes
/// from the rendered notes, or from all notes when none is rendered.
pub fn fill_degrees(notes: &mut [Note], key: Option<&Key>) {
    let Some(key) = key else {
        for n in notes.iter_mut() {
            n.degree = None;
        }
        return;
    };
    let root = root_for(notes, key);
    for n in notes.iter_mut() {
        n.degree = Some(scale_degree(n.midi, root, key));
    }
}

fn root_for(notes: &[Note], key: &Key) -> i32 {
    let shown: Vec<&Note> = rendered(notes).collect();
    if shown.is_empty() {
        scale_root_midi(&notes.iter().collect::<Vec<_>>(), key)
    } else {
        scale_root_midi(&shown, key)
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// One bar's events, `(length in steps, token)`, the token `None` for a rest.
fn bar_events<'a>(
    notes: &[&'a Note],
    bar: u32,
    steps_per_bar: u32,
) -> Vec<(u64, Option<&'a Note>)> {
    let spb = u64::from(steps_per_bar.max(1));
    let mut in_bar: Vec<&Note> = notes
        .iter()
        .copied()
        .filter(|n| n.bar == Some(bar))
        .collect();
    in_bar.sort_by_key(|n| n.step);
    let mut events = Vec::new();
    let mut cursor = 0u64;
    for n in in_bar {
        let (Some(step), Some(steps)) = (n.step, n.steps) else {
            continue;
        };
        let start = u64::from(step);
        // quantize never overlaps notes; a note starting before the cursor or past the bar
        // is left out, one running past the bar end is cut there
        if start < cursor || start >= spb || steps == 0 {
            continue;
        }
        if start > cursor {
            events.push((start - cursor, None));
        }
        let end = (start + u64::from(steps)).min(spb);
        events.push((end - start, Some(n)));
        cursor = end;
    }
    if cursor < spb {
        events.push((spb - cursor, None));
    }
    events
}

/// One bar as tokens: `token` names a note, rests are `~`, lengths reduced by their GCD.
fn bar_text(events: &[(u64, Option<&Note>)], token: &dyn Fn(&Note) -> String) -> String {
    if events.iter().all(|(_, n)| n.is_none()) {
        return "~".to_string();
    }
    let g = events.iter().fold(0, |g, (len, _)| gcd(g, *len)).max(1);
    events
        .iter()
        .map(|(len, n)| {
            let name = n.map_or_else(|| "~".to_string(), token);
            match len / g {
                1 => name,
                k => format!("{name}@{k}"),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Bars 1 to `bars` as one pattern: a flat sequence for one bar, `<[..] [..]>` for several.
fn pattern(bars: &[Vec<(u64, Option<&Note>)>], token: &dyn Fn(&Note) -> String) -> String {
    match bars {
        [] => "~".to_string(),
        [one] => bar_text(one, token),
        many => {
            let groups: Vec<String> = many
                .iter()
                .map(|events| {
                    let text = bar_text(events, token);
                    if text == "~" {
                        text
                    } else {
                        format!("[{text}]")
                    }
                })
                .collect();
            format!("<{}>", groups.join(" "))
        }
    }
}

/// The mini-notation of bars `first_bar` to `last_bar` of `notes` at `steps_per_bar` steps per
/// bar: note names always, scale degrees and the `.scale` argument when `key` is given.
/// `first_bar` 0 is the pickup bar before bar 1 (the caller leaves out the notes of bar 0 that
/// are not in it); notes outside the bars are left out, and no bars (`last_bar` below
/// `first_bar`) is `~`. The result names its `first_bar` and `last_bar`, so a
/// pattern of a span later in the memo does not start with empty bars.
pub fn render_notation(
    notes: &[Note],
    steps_per_bar: u32,
    first_bar: u32,
    last_bar: u32,
    key: Option<&Key>,
) -> Notation {
    let shown: Vec<&Note> = notes
        .iter()
        .filter(|n| n.step.is_some() && n.steps.is_some())
        .filter(|n| n.bar.is_some_and(|b| (first_bar..=last_bar).contains(&b)))
        .collect();
    let per_bar: Vec<Vec<(u64, Option<&Note>)>> = (first_bar..=last_bar)
        .map(|bar| bar_events(&shown, bar, steps_per_bar))
        .collect();
    let note = pattern(&per_bar, &|n: &Note| midi_to_name(n.midi));
    let (n, scale) = match key {
        Some(key) => {
            let root = root_for(notes, key);
            let n = pattern(&per_bar, &|n: &Note| scale_degree(n.midi, root, key));
            (Some(n), Some(scale_label(key, root)))
        }
        None => (None, None),
    };
    Notation {
        note,
        n,
        scale,
        first_bar,
        last_bar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::{KeySource, SCALE_STEPS, parse_key_override};
    use crate::report::Components;

    fn note(bar: u32, step: u32, steps: u32, midi: i32) -> Note {
        Note {
            start: 0.0,
            end: 0.0,
            bar: Some(bar),
            step: Some(step),
            steps: Some(steps),
            midi,
            name: midi_to_name(midi),
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
        }
    }

    fn key(s: &str) -> Key {
        parse_key_override(s).unwrap()
    }

    #[test]
    fn midi_to_name_range() {
        assert_eq!(midi_to_name(0), "c-1");
        assert_eq!(midi_to_name(12), "c0");
        assert_eq!(midi_to_name(60), "c4");
        assert_eq!(midi_to_name(61), "c#4");
        assert_eq!(midi_to_name(69), "a4");
        assert_eq!(midi_to_name(127), "g9");
        assert_eq!(midi_to_name(-1), "b-2");
        assert_eq!(midi_to_name(i32::MIN), "e-178956972");
    }

    #[test]
    fn gcd_reduction() {
        // 16 steps: c4 for 4, rest 4, e4 for 8
        let notes = [note(1, 0, 4, 60), note(1, 8, 8, 64)];
        assert_eq!(render_notation(&notes, 16, 1, 1, None).note, "c4 ~ e4@2");
        // lengths 2, 6, 8: GCD 2
        let notes = [note(1, 0, 2, 60), note(1, 2, 6, 62), note(1, 8, 8, 64)];
        assert_eq!(render_notation(&notes, 16, 1, 1, None).note, "c4 d4@3 e4@4");
        // a lone sixteenth keeps the full resolution, the trailing rest kept
        let notes = [note(1, 0, 1, 60), note(1, 1, 3, 62)];
        assert_eq!(render_notation(&notes, 16, 1, 1, None).note, "c4 d4@3 ~@12");
        // one note over the whole bar
        assert_eq!(
            render_notation(&[note(1, 0, 16, 60)], 16, 1, 1, None).note,
            "c4"
        );
    }

    #[test]
    fn whole_bar_rest_is_tilde() {
        let notes = [note(1, 0, 16, 60), note(3, 0, 8, 62)];
        assert_eq!(
            render_notation(&notes, 16, 1, 3, None).note,
            "<[c4] ~ [d4 ~]>"
        );
        assert_eq!(render_notation(&[], 16, 1, 1, None).note, "~");
        assert_eq!(render_notation(&[], 16, 1, 0, None).note, "~");
    }

    #[test]
    fn first_bar_starts_the_pattern() {
        let notes = [note(2, 0, 16, 59), note(5, 0, 16, 60), note(7, 0, 8, 62)];
        // bars 5 to 7: the note of bar 2 is outside and left out
        let n = render_notation(&notes, 16, 5, 7, None);
        assert_eq!(n.note, "<[c4] ~ [d4 ~]>");
        assert_eq!((n.first_bar, n.last_bar), (5, 7));
        // one bar is a flat sequence wherever it is
        let n = render_notation(&notes, 16, 7, 7, None);
        assert_eq!((n.note.as_str(), n.first_bar), ("d4 ~", 7));
        // bar 0 is the pickup bar, rendered as the first cycle; no bars at all is a rest
        let n = render_notation(&notes, 16, 1, 2, None);
        assert_eq!((n.note.as_str(), n.first_bar), ("<~ [b3]>", 1));
        let pickup = [note(0, 12, 4, 55), note(1, 0, 16, 60)];
        let n = render_notation(&pickup, 16, 0, 1, Some(&key("C:major")));
        assert_eq!((n.note.as_str(), n.first_bar), ("<[~@3 g3] [c4]>", 0));
        assert_eq!(n.n.as_deref(), Some("<[~@3 -3] [0]>"));
        assert_eq!(n.scale.as_deref(), Some("C4:major"));
        let n = render_notation(&notes, 16, 8, 7, None);
        assert_eq!((n.note.as_str(), n.first_bar), ("~", 8));
        // the degree form covers the same bars
        let n = render_notation(&notes, 16, 5, 7, Some(&key("C:major")));
        assert_eq!(n.n.as_deref(), Some("<[0] ~ [1 ~]>"));
    }

    #[test]
    fn multi_bar_uses_angle_brackets() {
        let notes = [
            note(1, 0, 4, 57),
            note(1, 4, 4, 60),
            note(1, 8, 8, 64),
            note(2, 4, 4, 67),
            note(2, 8, 4, 69),
        ];
        assert_eq!(
            render_notation(&notes, 16, 1, 2, None).note,
            "<[a3 c4 e4@2] [~ g4 a4 ~]>"
        );
        // 6/8 at 16: 12 steps per bar
        let notes = [note(1, 0, 6, 60), note(1, 6, 6, 62), note(2, 0, 12, 64)];
        assert_eq!(
            render_notation(&notes, 12, 1, 2, None).note,
            "<[c4 d4] [e4]>"
        );
    }

    #[test]
    fn one_bar_is_flat_sequence() {
        let notes = [note(1, 0, 4, 60), note(1, 4, 4, 62), note(1, 8, 4, 64)];
        let text = render_notation(&notes, 16, 1, 1, None).note;
        assert_eq!(text, "c4 d4 e4 ~");
        assert!(!text.contains('<') && !text.contains('['));
    }

    #[test]
    fn notes_outside_the_rendered_bars_are_left_out() {
        let mut unresolved = note(1, 0, 4, 65);
        unresolved.step = None;
        let notes = [
            note(0, 12, 4, 55), // before the origin
            unresolved,
            note(1, 4, 4, 60),
            note(3, 0, 4, 62), // past `bars`
        ];
        assert_eq!(render_notation(&notes, 16, 1, 1, None).note, "~ c4 ~@2");
    }

    #[test]
    fn sharp_names() {
        let notes = [note(1, 0, 8, 61), note(1, 8, 8, 70)];
        assert_eq!(render_notation(&notes, 16, 1, 1, None).note, "c#4 a#4");
    }

    #[test]
    fn degree_zero_is_root() {
        let c = key("C:major");
        assert_eq!(scale_degree(60, 60, &c), "0");
        assert_eq!(scale_degree(72, 60, &c), "7");
        assert_eq!(scale_degree(67, 60, &c), "4");
        let notes = [note(1, 0, 8, 60), note(1, 8, 8, 64)];
        let n = render_notation(&notes, 16, 1, 1, Some(&c));
        assert_eq!(n.n.as_deref(), Some("0 2"));
        assert_eq!(n.scale.as_deref(), Some("C4:major"));
    }

    #[test]
    fn out_of_scale_uses_sharp_of_degree_below() {
        let c = key("C:major");
        assert_eq!(scale_degree(63, 60, &c), "1#", "e-flat4 in C major");
        assert_eq!(scale_degree(66, 60, &c), "3#", "f-sharp4 in C major");
        assert_eq!(scale_degree(61, 60, &c), "0#");
        // a pentatonic gap of three semitones: e to g in C major pentatonic
        let p = key("C:major pentatonic");
        assert_eq!(scale_degree(65, 60, &p), "2#");
        assert_eq!(scale_degree(66, 60, &p), "2##");
        assert_eq!(scale_degree(67, 60, &p), "3");
        assert_eq!(scale_degree(72, 60, &p), "5");
    }

    #[test]
    fn negative_degrees() {
        let c = key("C:major");
        assert_eq!(scale_degree(59, 60, &c), "-1", "b3 below C4");
        assert_eq!(scale_degree(57, 60, &c), "-2", "a3");
        assert_eq!(scale_degree(58, 60, &c), "-2#", "a#3");
        assert_eq!(scale_degree(48, 60, &c), "-7", "c3");
        assert_eq!(scale_degree(47, 60, &c), "-8", "b2");
        // extreme values do not overflow
        assert_eq!(
            scale_degree(i32::MIN, i32::MAX, &c).chars().next(),
            Some('-')
        );
        assert!(!scale_degree(i32::MAX, i32::MIN, &c).starts_with('-'));
    }

    #[test]
    fn scale_root_octave_nearest_median() {
        // the proposal's example: a3 c4 e4 g4 a4 e4 in A minor, median e4, root A3
        let a = key("A:minor");
        let notes = [
            note(1, 0, 1, 57),
            note(1, 1, 1, 60),
            note(1, 2, 2, 64),
            note(1, 5, 1, 67),
            note(1, 6, 1, 69),
            note(1, 7, 2, 64),
        ];
        let n = render_notation(&notes, 16, 1, 1, Some(&a));
        assert_eq!(n.note, "a3 c4 e4@2 ~ g4 a4 e4@2 ~@7");
        assert_eq!(n.n.as_deref(), Some("0 2 4@2 ~ 6 7 4@2 ~@7"));
        assert_eq!(n.scale.as_deref(), Some("A3:minor"));
        // a high melody moves the root up: median c6, root C6
        let c = key("C:major");
        let high = [note(1, 0, 8, 84), note(1, 8, 8, 86)];
        assert_eq!(
            render_notation(&high, 16, 1, 1, Some(&c)).scale.as_deref(),
            Some("C6:major")
        );
        // the median itself is the root: root at it, not an octave below
        let on = [note(1, 0, 8, 62), note(1, 8, 4, 64), note(1, 12, 4, 60)];
        assert_eq!(
            render_notation(&on, 16, 1, 1, Some(&key("D:dorian")))
                .scale
                .as_deref(),
            Some("D4:dorian")
        );
        // the even count takes the lower middle value: 60 and 71, median 60, root B3
        let even = [note(1, 0, 8, 60), note(1, 8, 8, 71)];
        let n = render_notation(&even, 16, 1, 1, Some(&key("B:minor")));
        assert_eq!(n.scale.as_deref(), Some("B3:minor"));
        // c4 is a semitone above B3, outside B minor: degree 0 plus a sharp
        assert_eq!(n.n.as_deref(), Some("0# 7"));
        // a flat root keeps its spelling
        assert_eq!(
            render_notation(&on, 16, 1, 1, Some(&key("Bb:major")))
                .scale
                .as_deref(),
            Some("Bb3:major")
        );
    }

    #[test]
    fn scale_octave_follows_the_root_letter() {
        // tonal reads the octave by letter (Note.midi("Cb4") is 59, "B#3" is 60, "E#4" 65,
        // "Fb4" 64; @tonaljs/tonal as used by vendor/strudel/packages/tonal/tonal.mjs), so a
        // root spelled across the B/C boundary takes the octave of its letter
        let label = |root: &str, midis: &[i32]| {
            let notes: Vec<Note> = midis
                .iter()
                .enumerate()
                .map(|(i, &m)| note(1, i as u32, 1, m))
                .collect();
            render_notation(&notes, 16, 1, 1, Some(&key(&format!("{root}:major"))))
                .scale
                .unwrap()
        };
        // median 59 (B3 by MIDI): the root Cb is MIDI 59, which tonal writes Cb4
        assert_eq!(label("Cb", &[59, 61, 63]), "Cb4:major");
        assert_eq!(label("cb", &[59, 61, 63]), "Cb4:major");
        // median 60 (C4 by MIDI): the root B# is MIDI 60, which tonal writes B#3
        assert_eq!(label("B#", &[60, 62, 64]), "B#3:major");
        // E# and Fb do not cross an octave boundary
        assert_eq!(label("E#", &[65, 67, 69]), "E#4:major");
        assert_eq!(label("Fb", &[64, 66, 68]), "Fb4:major");
        assert_eq!(label("B", &[59, 61, 63]), "B3:major");
        assert_eq!(label("C", &[60, 62, 64]), "C4:major");
    }

    #[test]
    fn tied_notes_reattack() {
        // a c4 from step 12 of bar 1 to step 4 of bar 2, split by quantize
        let mut first = note(1, 12, 4, 60);
        first.tied_to_next = true;
        let notes = [first, note(2, 0, 4, 60), note(2, 4, 4, 62)];
        let n = render_notation(&notes, 16, 1, 2, Some(&key("C:major")));
        assert_eq!(n.note, "<[~@3 c4] [c4 d4 ~@2]>");
        assert_eq!(n.n.as_deref(), Some("<[~@3 0] [0 1 ~@2]>"));
    }

    #[test]
    fn no_key_gives_no_n_form() {
        let notes = [note(1, 0, 16, 60)];
        let n = render_notation(&notes, 16, 1, 1, None);
        assert_eq!(n.note, "c4");
        assert_eq!(n.n, None);
        assert_eq!(n.scale, None);
        let mut notes = notes.to_vec();
        notes[0].degree = Some("0".into());
        fill_degrees(&mut notes, None);
        assert_eq!(notes[0].degree, None);
    }

    #[test]
    fn degrees_for_every_scale_in_scale_steps() {
        for (name, steps) in SCALE_STEPS {
            let k = Key {
                name: format!("D {name}"),
                tonic_pc: 2,
                scale: name.to_string(),
                confidence: None,
                runner_up: None,
                correlation: None,
                source: KeySource::Flag,
            };
            for (i, s) in steps.iter().enumerate() {
                let midi = 62 + i32::from(*s);
                assert_eq!(scale_degree(midi, 62, &k), i.to_string(), "{name} step {i}");
                // and an octave up continues the count
                assert_eq!(
                    scale_degree(midi + 12, 62, &k),
                    (i + steps.len()).to_string(),
                    "{name} step {i} an octave up"
                );
            }
        }
    }

    #[test]
    fn fill_degrees_uses_the_rendered_median() {
        let c = key("C:major");
        let mut notes = vec![note(0, 0, 4, 40), note(1, 0, 8, 60), note(1, 8, 8, 64)];
        fill_degrees(&mut notes, Some(&c));
        // the bar 0 note does not move the root (C4); it gets its degree all the same
        let degrees: Vec<_> = notes.iter().map(|n| n.degree.clone().unwrap()).collect();
        assert_eq!(degrees, ["-12", "0", "2"]);
        // nothing rendered (an unresolved grid): every note decides
        let mut loose = vec![note(1, 0, 4, 48), note(1, 0, 4, 50)];
        for n in &mut loose {
            n.bar = None;
            n.step = None;
        }
        fill_degrees(&mut loose, Some(&c));
        assert_eq!(loose[0].degree.as_deref(), Some("0"));
        assert_eq!(loose[1].degree.as_deref(), Some("1"));
    }
}
