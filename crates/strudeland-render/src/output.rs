//! Output paths and the atomic write of a finished render.

use std::path::{Path, PathBuf};

use strudeland_project::ProjectError;

use crate::{ExportPreset, ExportRole, Format, LengthRequest, Progress, RenderError, RenderOutput};

/// The output format: `--format` when given, else the `--out` extension, else WAV. A `--format`
/// that disagrees with a known `--out` extension is a usage error (`--format`); an `--out`
/// extension that names no format, without `--format`, is a usage error (`--out`).
pub fn resolve_format(format: Option<Format>, out: Option<&Path>) -> Result<Format, RenderError> {
    let ext = out
        .and_then(Path::extension)
        .map(|e| e.to_string_lossy().into_owned());
    let from_ext = ext.as_deref().and_then(Format::from_extension);
    match (format, from_ext, ext.as_deref()) {
        (Some(f), Some(e), _) if f != e => Err(RenderError::Usage {
            flag: "--format",
            message: format!(
                "--format {f} disagrees with the extension of {}",
                out.map(Path::display)
                    .map_or_else(String::new, |d| d.to_string())
            ),
        }),
        (Some(f), _, _) => Ok(f),
        (None, Some(e), _) => Ok(e),
        (None, None, Some(other)) => Err(RenderError::Usage {
            flag: "--out",
            message: format!(
                "cannot tell the format from the extension {other:?} (wav, mp3, ogg or opus); pass --format"
            ),
        }),
        (None, None, None) => Ok(Format::Wav),
    }
}

/// Where a finished render is written: a given path, written as is.
#[derive(Debug)]
pub struct OutputTarget {
    path: PathBuf,
}

impl OutputTarget {
    /// A given path (`--out`): written as is; a missing parent directory is an error.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        OutputTarget { path: path.into() }
    }

    /// The file the render goes to.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The part of the default file name after `<stem>-` that tells a render's length apart, so a
/// short check never replaces the whole song: the slugged section name for `--section`,
/// `len<N>` for `--length N`, `loops<N>` for `--loops N`, `bars<from>-<to>` for a window; none
/// for the default clip.
pub(crate) fn render_suffix(length: &LengthRequest) -> Option<String> {
    match length {
        LengthRequest::Clip { loops: None } => None,
        LengthRequest::Clip { loops: Some(n) } => Some(format!("loops{n}")),
        LengthRequest::Section { name } => Some(name.clone()),
        LengthRequest::Length { bars } => Some(format!("len{bars}")),
        LengthRequest::Window {
            from_bar, to_bar, ..
        } => Some(format!("bars{from_bar}-{to_bar}")),
    }
}

/// The suffix for a preset asset in a default filename. Both the CLI and desktop use this rule.
/// A role outside the preset's role table is an error.
pub(crate) fn export_suffix(
    length: &LengthRequest,
    preset: ExportPreset,
    role: ExportRole,
) -> Result<String, RenderError> {
    let suffix = match (preset, role) {
        (ExportPreset::IntroLoop, ExportRole::Intro) => "intro",
        (ExportPreset::IntroLoop, ExportRole::Loop) => "loop",
        (ExportPreset::TaggedLoop, ExportRole::Single) => "tagged-loop",
        (ExportPreset::FoldedLoop, ExportRole::Loop) => "folded-loop",
        (ExportPreset::OneShot, ExportRole::Single) => "one-shot",
        _ => {
            return Err(RenderError::Render(
                "invalid output role for export preset".into(),
            ));
        }
    };
    Ok(if preset == ExportPreset::OneShot {
        render_suffix(length)
            .map_or_else(|| suffix.to_owned(), |length| format!("{length}-{suffix}"))
    } else {
        suffix.to_owned()
    })
}

/// Writes `out.bytes` to `target` atomically (an existing file is replaced without asking) and
/// emits `Progress::Written`. Returns the absolute path (the given one if it cannot be made
/// absolute). A missing parent directory is an `Io` error and nothing is created.
pub fn write_output(
    out: &RenderOutput,
    target: &OutputTarget,
    progress: &dyn Fn(Progress),
) -> Result<PathBuf, RenderError> {
    let path = target.path();
    strudeland_project::atomic_write_bytes(path, &out.bytes).map_err(|e| io_error(path, e))?;
    let written = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    progress(Progress::Written {
        path: written.clone(),
    });
    Ok(written)
}

/// A project error from the output step as [`RenderError::Io`]: its own path for an I/O error,
/// else `target` with the error's text.
pub(crate) fn io_error(target: &Path, e: ProjectError) -> RenderError {
    match e {
        ProjectError::Io { path, source } => RenderError::Io { path, source },
        other => RenderError::Io {
            path: target.to_path_buf(),
            source: std::io::Error::other(other.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::testing;

    fn output(bytes: &[u8]) -> RenderOutput {
        testing::output(bytes, &[])
    }

    #[test]
    fn writes_and_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        let events = RefCell::new(Vec::new());
        let target = OutputTarget::file(&path);
        let written =
            write_output(&output(b"one"), &target, &|p| events.borrow_mut().push(p)).unwrap();
        assert_eq!(written, path);
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        write_output(&output(b"two!"), &target, &|p| events.borrow_mut().push(p)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two!");
        assert_eq!(
            events.into_inner(),
            vec![
                Progress::Written { path: path.clone() },
                Progress::Written { path: path.clone() }
            ]
        );
        // Only the file: no temp file left.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_missing_parent_is_an_io_error_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("a.wav");
        let called = RefCell::new(false);
        let e = write_output(&output(b"x"), &OutputTarget::file(&path), &|_| {
            *called.borrow_mut() = true
        })
        .unwrap_err();
        match &e {
            RenderError::Io { path: p, .. } => assert_eq!(p, &path),
            other => panic!("{other:?}"),
        }
        assert!(!*called.borrow());
        assert!(!dir.path().join("missing").exists());
    }

    #[test]
    fn project_errors_become_io() {
        let e = io_error(
            Path::new("/p/renders/a.wav"),
            ProjectError::PathOutsideProject("x".into()),
        );
        assert_eq!(
            e.to_string(),
            "/p/renders/a.wav: path \"x\" is outside the project's songs folder"
        );
        assert!(matches!(e, RenderError::Io { .. }));
    }

    fn usage(r: Result<impl std::fmt::Debug, RenderError>) -> (&'static str, String) {
        match r {
            Err(RenderError::Usage { flag, message }) => (flag, message),
            other => panic!("not a usage error: {other:?}"),
        }
    }

    #[test]
    fn format_from_the_flag_the_extension_or_wav() {
        let p = |s: &'static str| Some(Path::new(s));
        for (flag, out, want) in [
            (None, None, Format::Wav),
            (None, p("a"), Format::Wav),
            (None, p("dir.d/a"), Format::Wav),
            (None, p("a.wav"), Format::Wav),
            (None, p("a.MP3"), Format::Mp3),
            (None, p("x/a.ogg"), Format::Ogg),
            (None, p("a.Opus"), Format::Opus),
            (Some(Format::Ogg), None, Format::Ogg),
            (Some(Format::Ogg), p("a.ogg"), Format::Ogg),
            (Some(Format::Opus), p("a"), Format::Opus),
            // An extension that names no format does not disagree with --format.
            (Some(Format::Mp3), p("a.bin"), Format::Mp3),
        ] {
            assert_eq!(resolve_format(flag, out).unwrap(), want, "{flag:?} {out:?}");
        }
    }

    #[test]
    fn a_format_that_disagrees_with_the_extension_is_a_usage_error() {
        assert_eq!(
            usage(resolve_format(
                Some(Format::Mp3),
                Some(Path::new("out/a.wav"))
            )),
            (
                "--format",
                "--format mp3 disagrees with the extension of out/a.wav".into()
            )
        );
        assert_eq!(
            usage(resolve_format(Some(Format::Opus), Some(Path::new("a.OGG")))),
            (
                "--format",
                "--format opus disagrees with the extension of a.OGG".into()
            )
        );
        assert_eq!(
            usage(resolve_format(None, Some(Path::new("a.flac")))),
            (
                "--out",
                "cannot tell the format from the extension \"flac\" (wav, mp3, ogg or opus); pass --format".into()
            )
        );
    }
}
