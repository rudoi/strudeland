//! The host's error type.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Why a host request failed. `Meta`, `Eval` and `Plan` carry the text of the bundle's result
/// slot whose `kind` matched (the song's `meta` header, its evaluation, or the query and plan);
/// `MetaFlag` is a `meta` failure of a render's length request: its typed [`LengthProblem`]
/// (`None` when the slot's problem does not parse, for example a code this crate does not know)
/// and the planner's message; `Timeout` is a planning that ran past its deadline;
/// `Unsettled` is an empty slot after the microtask drain (a promise still pending, for example
/// an `await` on a timer, which never fires); `Runtime` is everything outside the entry: the
/// bundle failing to load, an exception thrown outside the entry, a native failure, or a thread
/// or channel failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostError {
    #[error("meta: {0}")]
    Meta(String),
    #[error("meta: {message}")]
    MetaFlag {
        problem: Option<LengthProblem>,
        message: String,
    },
    #[error("eval: {0}")]
    Eval(String),
    #[error("plan: {0}")]
    Plan(String),
    #[error(
        "unsettled: the song's evaluation did not settle (it waits on a timer or a promise that never resolves)"
    )]
    Unsettled,
    #[error("runtime: {0}")]
    Runtime(String),
    #[error("timeout: planning did not finish within {} s", millis_as_secs(*.millis))]
    Timeout { millis: u64 },
}

/// A render's length request the song cannot satisfy (`LengthProblem` in `job-plan.ts`), as
/// the bundle's result slot carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "code",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum LengthProblem {
    /// A loop preset combined with a section, a length or a loop count.
    PresetWithLength,
    /// A loop count on a song that marks no loop.
    LoopsWithoutLoop,
    /// A loop count the structure refuses, with its message.
    LoopsInvalid { message: String },
    /// The whole clip of a song without sections.
    NoSections,
    /// A loop count that makes the clip longer than the render limit.
    LoopsPastLimit {
        loops: u32,
        clip_bars: u64,
        limit: u64,
    },
    /// The whole clip, longer than the render limit; `has_loop` when the song marks a loop.
    ClipPastLimit {
        clip_bars: u64,
        limit: u64,
        has_loop: bool,
    },
    /// A section of a song without sections.
    SectionWithoutSections,
    /// A section the song does not have, with the song's section names.
    UnknownSection { name: String, sections: Vec<String> },
    /// A section (or a single-section loop) that ends past the render limit.
    SectionPastLimit {
        name: String,
        to_bar: u64,
        limit: u64,
    },
    /// A length in bars past the end of the song's clip.
    LengthPastEnd { bars: u64, clip_bars: u64 },
    /// A loop preset on a song that marks no loop.
    PresetWithoutLoop,
    /// A loop preset whose loop range ends past the render limit.
    LoopPastLimit { end: u64, limit: u64 },
}

/// Whole milliseconds as seconds, for `Display`.
fn millis_as_secs(millis: u64) -> f64 {
    millis as f64 / 1000.0
}

/// The start of the bundle's plan error for a malformed render request.
pub const BAD_REQUEST_PREFIX: &str = "bad request:";

impl HostError {
    /// The lowercase kind, as `Display` and the binary print it.
    pub fn kind(&self) -> &'static str {
        match self {
            HostError::Meta(_) | HostError::MetaFlag { .. } => "meta",
            HostError::Eval(_) => "eval",
            HostError::Plan(_) => "plan",
            HostError::Unsettled => "unsettled",
            HostError::Runtime(_) => "runtime",
            HostError::Timeout { .. } => "timeout",
        }
    }

    /// Whether this is the bundle's `bad request: …` plan error (a malformed render request).
    pub fn is_bad_request(&self) -> bool {
        matches!(self, HostError::Plan(m) if m.starts_with(BAD_REQUEST_PREFIX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_kind_then_message() {
        let cases = [
            (HostError::Meta("m".into()), "meta", "meta: m"),
            (HostError::Eval("e".into()), "eval", "eval: e"),
            (HostError::Plan("p".into()), "plan", "plan: p"),
            (HostError::Runtime("r".into()), "runtime", "runtime: r"),
        ];
        for (err, kind, text) in cases {
            assert_eq!(err.kind(), kind, "{err:?}");
            assert_eq!(err.to_string(), text, "{err:?}");
        }
        let flagged = HostError::MetaFlag {
            problem: Some(LengthProblem::NoSections),
            message: "pass --length N".into(),
        };
        assert_eq!(flagged.kind(), "meta");
        assert_eq!(flagged.to_string(), "meta: pass --length N");
        for (millis, text) in [
            (500, "timeout: planning did not finish within 0.5 s"),
            (120_000, "timeout: planning did not finish within 120 s"),
            (1, "timeout: planning did not finish within 0.001 s"),
            (0, "timeout: planning did not finish within 0 s"),
        ] {
            let e = HostError::Timeout { millis };
            assert_eq!(e.kind(), "timeout");
            assert_eq!(e.to_string(), text);
        }
        assert_eq!(HostError::Unsettled.kind(), "unsettled");
        assert!(HostError::Unsettled.to_string().starts_with("unsettled: "));
    }

    #[test]
    fn only_a_bad_request_plan_error_is_a_bad_request() {
        assert!(HostError::Plan("bad request: --loops must be an integer".into()).is_bad_request());
        assert!(!HostError::Plan("query before a successful evaluate".into()).is_bad_request());
        assert!(!HostError::Meta("bad request: x".into()).is_bad_request());
        let flagged = HostError::MetaFlag {
            problem: Some(LengthProblem::LoopsWithoutLoop),
            message: "bad request: x".into(),
        };
        assert!(!flagged.is_bad_request());
    }

    /// One example of each code, as the TS planner writes it.
    fn examples() -> Vec<(&'static str, LengthProblem)> {
        vec![
            (
                r#"{"code":"presetWithLength"}"#,
                LengthProblem::PresetWithLength,
            ),
            (
                r#"{"code":"loopsWithoutLoop"}"#,
                LengthProblem::LoopsWithoutLoop,
            ),
            (
                r#"{"code":"loopsInvalid","message":"x"}"#,
                LengthProblem::LoopsInvalid {
                    message: "x".into(),
                },
            ),
            (r#"{"code":"noSections"}"#, LengthProblem::NoSections),
            (
                r#"{"code":"loopsPastLimit","loops":1000,"clipBars":10000002,"limit":100000}"#,
                LengthProblem::LoopsPastLimit {
                    loops: 1000,
                    clip_bars: 10000002,
                    limit: 100000,
                },
            ),
            (
                r#"{"code":"clipPastLimit","clipBars":110000,"limit":100000,"hasLoop":true}"#,
                LengthProblem::ClipPastLimit {
                    clip_bars: 110000,
                    limit: 100000,
                    has_loop: true,
                },
            ),
            (
                r#"{"code":"sectionWithoutSections"}"#,
                LengthProblem::SectionWithoutSections,
            ),
            (
                r#"{"code":"unknownSection","name":"drop","sections":["intro","body","tail"]}"#,
                LengthProblem::UnknownSection {
                    name: "drop".into(),
                    sections: vec!["intro".into(), "body".into(), "tail".into()],
                },
            ),
            (
                r#"{"code":"sectionPastLimit","name":"s10","toBar":110000,"limit":100000}"#,
                LengthProblem::SectionPastLimit {
                    name: "s10".into(),
                    to_bar: 110000,
                    limit: 100000,
                },
            ),
            (
                r#"{"code":"lengthPastEnd","bars":9,"clipBars":8}"#,
                LengthProblem::LengthPastEnd {
                    bars: 9,
                    clip_bars: 8,
                },
            ),
            (
                r#"{"code":"presetWithoutLoop"}"#,
                LengthProblem::PresetWithoutLoop,
            ),
            (
                r#"{"code":"loopPastLimit","end":110000,"limit":100000}"#,
                LengthProblem::LoopPastLimit {
                    end: 110000,
                    limit: 100000,
                },
            ),
        ]
    }

    #[test]
    fn every_length_problem_round_trips() {
        let examples = examples();
        assert_eq!(examples.len(), 12);
        for (json, problem) in &examples {
            let read: LengthProblem = serde_json::from_str(json).expect(json);
            assert_eq!(&read, problem, "{json}");
            assert_eq!(serde_json::to_string(&read).unwrap(), *json);
        }
    }

    #[test]
    fn an_unknown_code_or_a_bad_param_is_rejected() {
        for json in [
            r#"{"code":"nope"}"#,
            r#"{"code":"lengthPastEnd","bars":-1,"clipBars":8}"#,
            r#"{"code":"lengthPastEnd","bars":9}"#,
            r#"{"code":"clipPastLimit","clipBars":1.5,"limit":100000,"hasLoop":true}"#,
        ] {
            assert!(
                serde_json::from_str::<LengthProblem>(json).is_err(),
                "{json}"
            );
        }
    }
}
