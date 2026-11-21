//! The serializable result of a transcription. Field names are camelCase and optional fields
//! serialize as `null`, except the debug fields, which are omitted unless set.

use serde::{Deserialize, Serialize};

pub use crate::grid::Resolved;
pub use crate::key::{Key, KeySource, NoKey};
pub use crate::onset::Onset;
use crate::quantize::GridDivision;

/// The whole result: the grid, the tuning, the notes and their notation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcription {
    /// The memo's name; set by the CLI, null from the crate.
    pub memo: Option<String>,
    /// Seconds analysed.
    pub duration: f64,
    /// The resolved grid; null when it could not be resolved.
    pub grid: Option<GridReport>,
    pub tuning: TuningReport,
    /// The detected or given key; null when none was detected and none given.
    pub key: Option<Key>,
    /// Null for no notes or no grid.
    pub notation: Option<Notation>,
    pub notes: Vec<Note>,
    pub dropped: Vec<Dropped>,
    /// Milliseconds, the 90th percentile of `|snapErrorMs|`.
    pub timing_spread: Option<f32>,
    pub warnings: Vec<String>,
    /// Why detection chose no key (its warning in `warnings` says the same); not serialized.
    #[serde(skip)]
    pub no_key: Option<NoKey>,
    /// Filled by the CLI from the sidecar; null from the crate.
    pub backing: Option<BackingReport>,
    /// Pitch frames, with `--debug` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<DebugFrame>>,
    /// Onset peaks (seconds from the start of the analysed span), with `--debug` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onsets: Option<Vec<Onset>>,
}

/// The grid the notes were quantized to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridReport {
    /// Quarter notes per minute.
    pub bpm: f64,
    /// `"N/D"`.
    pub meter: String,
    /// Quarter notes per bar.
    pub beats_per_bar: f64,
    /// Bar 1's downbeat, seconds in the memo.
    pub origin: f64,
    pub source: GridSource,
    /// The minimum of the inferred parts' confidences; null when nothing was inferred.
    pub confidence: Option<f32>,
    pub steps_per_bar: u32,
    /// The `--grid` value: `"4"`, `"8"`, `"16"`, `"8t"` or `"16t"`.
    pub division: GridDivision,
    /// A `--grid` value that fits the rhythm clearly better; null otherwise (a suggestion only).
    pub suggested_division: Option<GridDivision>,
    /// A `--shift` (the whole flag value, milliseconds) that brings the notes clearly closer to
    /// the grid: on `suggested_division` when that is set (use the two together), else on
    /// `division`; null otherwise (a suggestion only).
    pub suggested_shift_ms: Option<f64>,
}

/// Where the tempo and the phase came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GridSource {
    pub tempo: Resolved,
    pub phase: Resolved,
}

/// The tuning offset: positive means the singer is sharp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TuningReport {
    pub cents: f32,
    pub source: TuningSource,
}

/// Where the tuning came from; serialized as `"estimated"` or `"flag"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TuningSource {
    /// Estimated from the take.
    Estimated,
    /// `--tuning`.
    Flag,
}

impl TuningSource {
    /// The report's name: `"estimated"` or `"flag"`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Estimated => "estimated",
            Self::Flag => "flag",
        }
    }
}

/// The proposed Strudel code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notation {
    /// Mini-notation of note names.
    pub note: String,
    /// Mini-notation of scale degrees; null without a key.
    pub n: Option<String>,
    /// The `.scale(...)` argument; null without a key.
    pub scale: Option<String>,
    /// The memo bar the pattern's first cycle is: the bar the analysed span starts in, at least 1,
    /// or 0 (the pickup bar) when the span starts before bar 1 and has notes in that bar.
    pub first_bar: u32,
    /// The memo bar the pattern's last cycle is: the last bar with a note.
    pub last_bar: u32,
}

/// One transcribed note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// Raw detected start, seconds in the memo.
    pub start: f64,
    /// Raw detected end, seconds in the memo.
    pub end: f64,
    /// The memo's bar, 1-based; 0 before the origin; null when the grid is unresolved.
    pub bar: Option<u32>,
    /// The quantized start step in the bar; null when the grid is unresolved.
    pub step: Option<u32>,
    /// The quantized length in steps; null when the grid is unresolved.
    pub steps: Option<u32>,
    /// The integer semitone.
    pub midi: i32,
    /// `midi` as a lowercase note name, e.g. `c#4`.
    pub name: String,
    /// The scale degree in the key; null without a key.
    pub degree: Option<String>,
    /// The note's median deviation from its semitone after tuning, -50..50.
    pub cents: f32,
    pub confidence: f32,
    pub components: Components,
    /// Quantized start minus raw start, milliseconds.
    pub snap_error_ms: Option<f32>,
    /// True when this part of a sustained note continues into the next bar.
    pub tied_to_next: bool,
    /// The neighbouring semitone the note leans toward when its `cents` are beyond
    /// `AMBIGUOUS_CENTS` either way (the pitch is ambiguous between the two); null otherwise.
    pub alternative: Option<Alternative>,
    /// True for a part of a note that starts more than one bar before the origin (bar 0, but
    /// before the pickup bar), which the notation leaves out; not in the JSON.
    #[serde(skip)]
    pub(crate) early: bool,
}

/// The other semitone an ambiguous note may be.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alternative {
    pub midi: i32,
    /// `midi` as a lowercase note name.
    pub name: String,
}

impl Alternative {
    /// The semitone a note of `midi` leans toward at `cents` (positive is sharp) when they are
    /// beyond `AMBIGUOUS_CENTS`; `None` otherwise or when not finite.
    pub fn of(midi: i32, cents: f32) -> Option<Self> {
        if !(cents.is_finite() && cents.abs() > crate::consts::AMBIGUOUS_CENTS) {
            return None;
        }
        let other = midi + if cents > 0.0 { 1 } else { -1 };
        Some(Self {
            midi: other,
            name: crate::notation::midi_to_name(other),
        })
    }
}

/// What the confidence is made of, each 0..=1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Components {
    pub voiced: f32,
    pub stability: f32,
    pub fit: f32,
    /// The onset strength for a note split by an onset, else null.
    pub onset: Option<f32>,
}

/// A note quantizing removed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dropped {
    pub start: f64,
    pub end: f64,
    pub midi: i32,
    pub name: String,
    pub reason: String,
}

/// What the take was recorded against, from the sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackingReport {
    pub song: Option<String>,
    pub start_section: Option<String>,
}

/// One pitch frame, with `--debug` only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugFrame {
    /// Seconds from the start of the analysed span.
    pub t: f64,
    pub f0: Option<f32>,
    pub voiced: f32,
    pub rms: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// The proposal's JSON example.
    const EXAMPLE: &str = r#"{"memo":"main-01","duration":12.4,
 "grid":{"bpm":100.0,"meter":"4/4","beatsPerBar":4.0,"origin":2.4,
         "source":{"tempo":"sidecar","phase":"sidecar"},"confidence":null,
         "stepsPerBar":16,"division":"16","suggestedDivision":null,"suggestedShiftMs":null},
 "tuning":{"cents":-12.0,"source":"estimated"},
 "key":{"name":"A minor","tonicPc":9,"scale":"minor","confidence":0.18,"runnerUp":"C major",
        "correlation":0.82,"source":"detected"},
 "notation":{"note":"a3 c4 e4@2 ~ g4 a4 e4@2 ~@4","n":"0 2 4@2 ~ 6 7 4@2 ~@4","scale":"A3:minor",
             "firstBar":1,"lastBar":1},
 "notes":[{"start":2.43,"end":2.97,"bar":1,"step":0,"steps":1,"midi":57,"name":"a3",
           "degree":"0","cents":3.1,"confidence":0.91,
           "components":{"voiced":0.97,"stability":0.93,"fit":0.94,"onset":null},
           "snapErrorMs":-14.0,"tiedToNext":false,"alternative":null}],
 "dropped":[],"timingSpread":18.5,"warnings":[],
 "backing":{"song":"songs/main.js","startSection":"verse"}}"#;

    fn example() -> Transcription {
        serde_json::from_str(EXAMPLE).expect("the example parses")
    }

    fn minimal() -> Transcription {
        Transcription {
            memo: None,
            duration: 0.0,
            grid: None,
            tuning: TuningReport {
                cents: 0.0,
                source: TuningSource::Estimated,
            },
            key: None,
            notation: None,
            notes: vec![],
            dropped: vec![],
            timing_spread: None,
            warnings: vec![],
            no_key: None,
            backing: None,
            frames: None,
            onsets: None,
        }
    }

    #[test]
    fn json_field_names_are_camel_case() {
        let t = example();
        assert_eq!(t.memo.as_deref(), Some("main-01"));
        assert_eq!(t.duration, 12.4);
        let grid = t.grid.as_ref().expect("grid");
        assert_eq!(grid.beats_per_bar, 4.0);
        assert_eq!(grid.steps_per_bar, 16);
        assert_eq!(grid.source.tempo, Resolved::Sidecar);
        assert_eq!(grid.division, GridDivision::Sixteenth);
        assert_eq!(t.tuning.source, TuningSource::Estimated);
        assert_eq!(TuningSource::Flag.label(), "flag");
        assert_eq!(serde_json::to_value(TuningSource::Flag).unwrap(), "flag");
        assert_eq!(grid.confidence, None);
        assert_eq!(t.tuning.cents, -12.0);
        let key = t.key.as_ref().expect("key");
        assert_eq!(key.name, "A minor");
        assert_eq!(key.tonic_pc, 9);
        assert_eq!(key.scale, "minor");
        assert_eq!(key.confidence, Some(0.18));
        assert_eq!(key.runner_up.as_deref(), Some("C major"));
        assert_eq!(key.correlation, Some(0.82));
        assert_eq!(key.source, KeySource::Detected);
        assert_eq!(t.timing_spread, Some(18.5));
        let notation = t.notation.as_ref().expect("notation");
        assert_eq!((notation.first_bar, notation.last_bar), (1, 1));
        let note = &t.notes[0];
        assert_eq!(note.snap_error_ms, Some(-14.0));
        assert!(!note.tied_to_next);
        assert_eq!(note.components.onset, None);
        let backing = t.backing.as_ref().expect("backing");
        assert_eq!(backing.start_section.as_deref(), Some("verse"));
        // serializing gives exactly the example's names and values
        let expected: Value = serde_json::from_str(EXAMPLE).unwrap();
        // through text, so f32 fields print as their shortest decimal
        let text = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), expected);
    }

    #[test]
    fn debug_fields_omitted_unless_set() {
        let v = serde_json::to_value(minimal()).unwrap();
        assert!(v.get("frames").is_none(), "frames omitted when None");
        assert!(v.get("onsets").is_none(), "onsets omitted when None");
        let mut t = minimal();
        t.frames = Some(vec![DebugFrame {
            t: 0.01,
            f0: Some(220.0),
            voiced: 0.5,
            rms: 0.25,
        }]);
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(
            v["frames"],
            json!([{"t":0.01,"f0":220.0,"voiced":0.5,"rms":0.25}])
        );
        assert!(v.get("onsets").is_none(), "onsets still omitted");
        let mut t = minimal();
        t.onsets = Some(vec![Onset {
            seconds: 1.25,
            strength: 0.5,
        }]);
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["onsets"], json!([{"seconds":1.25,"strength":0.5}]));
        assert!(v.get("frames").is_none(), "frames still omitted");
        t.onsets = Some(vec![]);
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["onsets"], json!([]), "an empty list is set, so present");
    }

    #[test]
    fn no_key_reason_is_not_serialized() {
        let mut t = minimal();
        t.no_key = Some(NoKey::TooClose);
        assert_eq!(
            serde_json::to_string(&t).unwrap(),
            serde_json::to_string(&minimal()).unwrap()
        );
    }

    #[test]
    fn nulls_not_omitted() {
        let v = serde_json::to_value(minimal()).unwrap();
        for key in ["memo", "grid", "key", "notation", "timingSpread", "backing"] {
            assert!(v[key].is_null(), "{key} is present and null");
        }
        let mut t = example();
        t.notes[0].bar = None;
        t.notes[0].step = None;
        t.notes[0].steps = None;
        t.notes[0].degree = None;
        t.notes[0].snap_error_ms = None;
        let v = serde_json::to_value(&t).unwrap();
        for key in ["bar", "step", "steps", "degree", "snapErrorMs"] {
            assert!(
                v["notes"][0][key].is_null(),
                "note.{key} is present and null"
            );
        }
        assert!(v["notes"][0]["components"]["onset"].is_null());
    }

    #[test]
    fn alternative_names_the_semitone_a_note_leans_toward() {
        assert_eq!(
            Alternative::of(48, 31.0).map(|a| (a.midi, a.name)),
            Some((49, "c#3".into()))
        );
        assert_eq!(
            Alternative::of(48, -45.0).map(|a| (a.midi, a.name)),
            Some((47, "b2".into()))
        );
        assert_eq!(Alternative::of(48, 30.0), None, "30 cents is not ambiguous");
        assert_eq!(Alternative::of(48, -12.0), None);
        assert_eq!(Alternative::of(48, f32::NAN), None);
        let mut t = example();
        t.notes[0].alternative = Alternative::of(57, -40.0);
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(
            v["notes"][0]["alternative"],
            serde_json::json!({"midi": 56, "name": "g#3"})
        );
    }

    #[test]
    fn round_trips_through_serde_json() {
        let mut t = example();
        t.frames = Some(vec![DebugFrame {
            t: 0.5,
            f0: None,
            voiced: 0.0,
            rms: 0.0,
        }]);
        t.onsets = Some(vec![Onset {
            seconds: 0.5,
            strength: 1.0,
        }]);
        t.dropped.push(Dropped {
            start: 1.0,
            end: 1.1,
            midi: 60,
            name: "c4".into(),
            reason: "shorter note on the same step".into(),
        });
        let text = serde_json::to_string(&t).unwrap();
        let back: Transcription = serde_json::from_str(&text).unwrap();
        assert_eq!(back, t);
        assert_eq!(serde_json::to_string(&back).unwrap(), text);
    }
}
