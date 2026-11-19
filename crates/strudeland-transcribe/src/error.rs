//! The crate's error type and its exit codes.

use crate::grid::Resolved;

/// Why a transcription or a load failed.
#[derive(Debug, thiserror::Error)]
pub enum TranscribeError {
    /// The file is unreadable or not a WAV (exit 1).
    #[error("cannot read {path}: {message}")]
    Wav { path: String, message: String },
    /// The bar 1 the grid would use (`from` the sidecar or `--bar1`) lies beyond the audio
    /// (exit 1).
    #[error(
        "bar 1 ({bar1:.3} s, from {}) is past the end of the audio ({len:.3} s)",
        bar1_source(*.from)
    )]
    Bar1PastEnd { bar1: f64, len: f64, from: Resolved },
    /// A whole-file read of a memo over the cap (exit 4).
    #[error(
        "the memo is {seconds:.0} s long; pass --start/--end or --bars to pick up to 10 minutes"
    )]
    TooLong { seconds: f64 },
    /// A span that is empty, negative, not a number or past the end (exit 4).
    #[error("{0}")]
    BadSpan(String),
    /// `--bars` without a known grid (exit 4).
    #[error("--bars needs a known grid (sidecar bar 1, or --bpm, --meter and --bar1)")]
    BarsNeedGrid,
    /// A grid that does not split the bar into whole steps (exit 4).
    #[error(
        "--grid {division} does not divide a {meter} bar into whole steps ({steps} steps); try a finer grid"
    )]
    GridDoesNotDivide {
        division: String,
        meter: String,
        steps: f64,
    },
    /// A malformed or unknown key (exit 4).
    #[error("{0}")]
    BadKey(String),
    /// Any other option the crate cannot satisfy (exit 4).
    #[error("{0}")]
    BadOption(String),
}

/// Where a bar 1 came from, as `Bar1PastEnd` names it.
fn bar1_source(from: Resolved) -> &'static str {
    match from {
        Resolved::Flag => "--bar1",
        Resolved::Sidecar => "the sidecar",
        Resolved::Inferred => "the audio",
    }
}

impl TranscribeError {
    /// The CLI's exit code: 1 for a failure of the memo itself (unreadable, or its bar 1 past the
    /// end), 4 for a usage error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Wav { .. } | Self::Bar1PastEnd { .. } => 1,
            Self::TooLong { .. }
            | Self::BadSpan(_)
            | Self::BarsNeedGrid
            | Self::GridDoesNotDivide { .. }
            | Self::BadKey(_)
            | Self::BadOption(_) => 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_variants() -> Vec<TranscribeError> {
        vec![
            TranscribeError::TooLong { seconds: 1832.0 },
            TranscribeError::BadSpan("bad".into()),
            TranscribeError::BarsNeedGrid,
            TranscribeError::GridDoesNotDivide {
                division: "4".into(),
                meter: "3/8".into(),
                steps: 1.5,
            },
            TranscribeError::BadKey("bad".into()),
            TranscribeError::BadOption("bad".into()),
        ]
    }

    #[test]
    fn exit_code_4_for_usage_variants() {
        for e in usage_variants() {
            assert_eq!(e.exit_code(), 4, "{e:?} should be a usage error");
        }
    }

    #[test]
    fn exit_code_1_for_wav_and_bar1_past_end() {
        let wav = TranscribeError::Wav {
            path: "a.wav".into(),
            message: "nope".into(),
        };
        let bar1 = TranscribeError::Bar1PastEnd {
            bar1: 3.0,
            len: 2.0,
            from: Resolved::Flag,
        };
        assert_eq!(wav.exit_code(), 1);
        assert_eq!(bar1.exit_code(), 1);
    }

    #[test]
    fn display_strings_match_the_table() {
        let cases: Vec<(TranscribeError, &str)> = vec![
            (
                TranscribeError::Wav {
                    path: "recordings/a.wav".into(),
                    message: "no such file".into(),
                },
                "cannot read recordings/a.wav: no such file",
            ),
            (
                TranscribeError::Bar1PastEnd {
                    bar1: 14.0,
                    len: 12.4,
                    from: Resolved::Sidecar,
                },
                "bar 1 (14.000 s, from the sidecar) is past the end of the audio (12.400 s)",
            ),
            (
                TranscribeError::Bar1PastEnd {
                    bar1: 14.0,
                    len: 12.4,
                    from: Resolved::Flag,
                },
                "bar 1 (14.000 s, from --bar1) is past the end of the audio (12.400 s)",
            ),
            (
                TranscribeError::TooLong { seconds: 1832.0 },
                "the memo is 1832 s long; pass --start/--end or --bars to pick up to 10 minutes",
            ),
            (TranscribeError::BadSpan("span".into()), "span"),
            (
                TranscribeError::BarsNeedGrid,
                "--bars needs a known grid (sidecar bar 1, or --bpm, --meter and --bar1)",
            ),
            (
                TranscribeError::GridDoesNotDivide {
                    division: "4".into(),
                    meter: "3/8".into(),
                    steps: 1.5,
                },
                "--grid 4 does not divide a 3/8 bar into whole steps (1.5 steps); try a finer grid",
            ),
            (
                TranscribeError::BadKey("bad --key \"X\"; use ROOT:SCALE such as C:major".into()),
                "bad --key \"X\"; use ROOT:SCALE such as C:major",
            ),
            (TranscribeError::BadOption("opt".into()), "opt"),
        ];
        for (err, text) in cases {
            assert_eq!(err.to_string(), text, "{err:?}");
        }
    }
}
