//! The host's error type.

use thiserror::Error;

/// Why a host request failed. `Meta`, `Eval` and `Plan` carry the text of the bundle's result
/// slot whose `kind` matched (the song's `meta` header, its evaluation, or the query and plan);
/// `MetaFlag` is a `meta` failure of a render's length request that names the CLI flag at fault
/// (`--length`, `--section`, `--loops`, `--preset`); `Timeout` is a planning that ran past its deadline;
/// `Unsettled` is an empty slot after the microtask drain (a promise still pending, for example
/// an `await` on a timer, which never fires); `Runtime` is everything outside the entry: the
/// bundle failing to load, an exception thrown outside the entry, a native failure, or a thread
/// or channel failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HostError {
    #[error("meta: {0}")]
    Meta(String),
    #[error("meta: {message}")]
    MetaFlag { flag: String, message: String },
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

/// Whole milliseconds as seconds, for `Display`.
fn millis_as_secs(millis: u64) -> f64 {
    millis as f64 / 1000.0
}

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
            flag: "--length".into(),
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
}
