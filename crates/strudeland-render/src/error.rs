//! Why a render failed, and how the CLI classes it.

use std::path::PathBuf;

use strudeland_host::HostError;

/// Why a render failed. Every variant names its stage; there is no `From` that hides one.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// Planning failed (any `HostError`, including `MetaFlag` and `Timeout`). Display is the
    /// host error's text, except a `MetaFlag` prints just its message, the planner's advice for
    /// the user, without the `meta:` prefix ("pass --length N ...").
    #[error("{}", plan_text(.0))]
    Plan(#[from] HostError),
    /// Nothing was rendered: the song uses material the engine cannot play (sorted, one line
    /// each in the list's order).
    #[error("{}", display_unsupported(.0))]
    Unsupported(Vec<Unsupported>),
    /// A chunk failed to render (a `JobError`, a surviving `Unsupported` render error, a short
    /// buffer, a failed internal check).
    #[error("render failed: {0}")]
    Render(String),
    #[error("encode failed: {0}")]
    Encode(String),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A bad option. `flag` is the CLI flag concerned (`--jobs`, `--quality`, ...).
    #[error("{message}")]
    Usage { flag: &'static str, message: String },
}

/// One unsupported signature of a refused song. Ordered by bar, then sound, then keys (the
/// field order): the order the refusal lists them in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unsupported {
    /// 0-based layout bar of the first hap (the minimum over chunks).
    pub layout_bar: i64,
    pub s: String,
    pub keys: Vec<String>,
    /// The section that bar plays in (clip: the clip arrangement; linear: the song's sections);
    /// `None` when the song has no section there.
    pub section: Option<String>,
}

/// How the CLI exits on an error: a usage error (exit 4) or a failed render (exit 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitClass {
    Usage,
    Failed,
}

impl RenderError {
    /// A [`RenderError::Render`] with `message`.
    pub(crate) fn render(message: impl Into<String>) -> RenderError {
        RenderError::Render(message.into())
    }

    /// `Usage` for [`RenderError::Usage`] and a planning failure of the length request
    /// ([`HostError::MetaFlag`]); `Failed` for everything else.
    pub fn exit_class(&self) -> ExitClass {
        match self {
            RenderError::Usage { .. } | RenderError::Plan(HostError::MetaFlag { .. }) => {
                ExitClass::Usage
            }
            _ => ExitClass::Failed,
        }
    }
}

fn plan_text(e: &HostError) -> String {
    match e {
        HostError::MetaFlag { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// The refusal message: a first line, then one line per signature sorted by bar, then sound.
fn display_unsupported(list: &[Unsupported]) -> String {
    let mut out = String::from(
        "the song uses sounds or controls the engine cannot play yet; nothing was written:",
    );
    for u in list {
        out.push_str("\n  s=");
        out.push_str(&u.s);
        if !u.keys.is_empty() {
            out.push_str(" keys=");
            out.push_str(&u.keys.join(","));
        }
        out.push_str(&format!(" first at bar {}", u.layout_bar.saturating_add(1)));
        if let Some(name) = &u.section {
            out.push_str(&format!(" (section '{name}')"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use strudeland_host::LengthProblem;

    fn unsupported(s: &str, keys: &[&str], bar: i64, section: Option<&str>) -> Unsupported {
        Unsupported {
            s: s.into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            layout_bar: bar,
            section: section.map(str::to_string),
        }
    }

    #[test]
    fn display_and_exit_class_of_every_variant() {
        let cases: Vec<(RenderError, &str, ExitClass)> = vec![
            (
                RenderError::Plan(HostError::Eval("boom".into())),
                "eval: boom",
                ExitClass::Failed,
            ),
            (
                RenderError::Plan(HostError::Meta("bad meta".into())),
                "meta: bad meta",
                ExitClass::Failed,
            ),
            (
                RenderError::Plan(HostError::MetaFlag {
                    problem: LengthProblem::NoSections,
                    message: "pass --length N".into(),
                }),
                "pass --length N",
                ExitClass::Usage,
            ),
            (
                RenderError::Plan(HostError::Timeout { millis: 1500 }),
                "timeout: planning did not finish within 1.5 s",
                ExitClass::Failed,
            ),
            (
                RenderError::Plan(HostError::Plan("p".into())),
                "plan: p",
                ExitClass::Failed,
            ),
            (
                RenderError::Plan(HostError::Runtime("r".into())),
                "runtime: r",
                ExitClass::Failed,
            ),
            (
                RenderError::Unsupported(vec![unsupported("zzfx", &[], 0, None)]),
                "the song uses sounds or controls the engine cannot play yet; nothing was written:\n  s=zzfx first at bar 1",
                ExitClass::Failed,
            ),
            (
                RenderError::Render("chunk 2: x".into()),
                "render failed: chunk 2: x",
                ExitClass::Failed,
            ),
            (
                RenderError::Encode("lame".into()),
                "encode failed: lame",
                ExitClass::Failed,
            ),
            (
                RenderError::Io {
                    path: PathBuf::from("/x/y.wav"),
                    source: std::io::Error::other("denied"),
                },
                "/x/y.wav: denied",
                ExitClass::Failed,
            ),
            (
                RenderError::Usage {
                    flag: "--jobs",
                    message: "--jobs must be at least 1".into(),
                },
                "--jobs must be at least 1",
                ExitClass::Usage,
            ),
        ];
        for (err, text, class) in cases {
            assert_eq!(err.to_string(), text, "{err:?}");
            assert_eq!(err.exit_class(), class, "{err:?}");
        }
        assert!(
            RenderError::Plan(HostError::Unsettled)
                .to_string()
                .starts_with("unsettled: ")
        );
    }

    #[test]
    fn unsupported_lines_name_the_sound_keys_bar_and_section() {
        let e = RenderError::Unsupported(vec![
            unsupported("hat", &["bank", "speed"], 0, Some("intro")),
            unsupported("kick", &[], 4, Some("body")),
            unsupported("tone", &["speed"], 4, Some("body")),
            unsupported("zzfx", &[], 11, None),
        ]);
        assert_eq!(
            e.to_string(),
            "the song uses sounds or controls the engine cannot play yet; nothing was written:\n  s=hat keys=bank,speed first at bar 1 (section 'intro')\n  s=kick first at bar 5 (section 'body')\n  s=tone keys=speed first at bar 5 (section 'body')\n  s=zzfx first at bar 12"
        );
    }
}
