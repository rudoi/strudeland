//! Output paths and the atomic write of a finished render.

use std::path::{Path, PathBuf};

use strudeland_project::{Project, ProjectError};

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

/// Where a finished render is written: a given path, or a file in a project's `renders/`.
#[derive(Debug)]
pub struct OutputTarget {
    path: PathBuf,
    /// The project whose `renders/` holds `path` (created when the file is written).
    project: Option<Project>,
}

impl OutputTarget {
    /// A given path (`--out`): written as is; a missing parent directory is an error.
    pub fn file(path: impl Into<PathBuf>) -> Self {
        OutputTarget {
            path: path.into(),
            project: None,
        }
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
pub fn render_suffix(length: &LengthRequest) -> Option<String> {
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

/// Where a render goes: `out` unchanged when given (a relative path is relative to the working
/// directory); else, for a song in a strudeland project, `renders/<stem>[-<suffix>].<ext>` in
/// that project ([`render_suffix`] of `length`). Creates nothing (the project's `renders/` is
/// created by [`write_output`]). A song outside a project needs `out` (a usage error naming
/// `--out`).
pub fn resolve_output(
    out: Option<&Path>,
    song: &Path,
    length: &LengthRequest,
    format: Format,
) -> Result<OutputTarget, RenderError> {
    if let Some(out) = out {
        return Ok(OutputTarget::file(out));
    }
    let suffix = render_suffix(length);
    project_output(song, suffix.as_deref(), format)
}

/// Resolves a preset destination without creating files. For split exports, `out` supplies
/// the stem and directory and each role gets its own suffix.
pub fn resolve_export_output(
    out: Option<&Path>,
    song: &Path,
    length: &LengthRequest,
    format: Format,
    preset: ExportPreset,
    role: ExportRole,
) -> Result<OutputTarget, RenderError> {
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
    if let Some(path) = out {
        if preset != ExportPreset::IntroLoop {
            return Ok(OutputTarget::file(path));
        }
        let mut name = path
            .file_stem()
            .ok_or_else(|| RenderError::Usage {
                flag: "--out",
                message: "the split export needs a file name for --out".into(),
            })?
            .to_os_string();
        name.push(format!("-{suffix}.{}", format.ext()));
        return Ok(OutputTarget::file(path.with_file_name(name)));
    }
    let suffix = if preset == ExportPreset::OneShot {
        render_suffix(length)
            .map_or_else(|| suffix.to_owned(), |length| format!("{length}-{suffix}"))
    } else {
        suffix.to_owned()
    };
    project_output(song, Some(&suffix), format)
}

fn project_output(
    song: &Path,
    suffix: Option<&str>,
    format: Format,
) -> Result<OutputTarget, RenderError> {
    let Some((project, stem)) = Project::find(song) else {
        return Err(RenderError::Usage {
            flag: "--out",
            message: format!(
                "--out is required: {} is not in a strudeland project",
                song.display()
            ),
        });
    };
    let path = project
        .render_path(&stem, suffix, format.ext())
        .map_err(|e| io_error(&project.root().join("renders"), e))?;
    Ok(OutputTarget {
        path,
        project: Some(project),
    })
}

/// Writes `out.bytes` to `target` atomically (an existing file is replaced without asking) and
/// emits `Progress::Written`. Returns the absolute path (the given one if it cannot be made
/// absolute). For a given path a missing parent directory is an `Io` error and nothing is
/// created; a project's `renders/` is created when missing.
pub fn write_output(
    out: &RenderOutput,
    target: &OutputTarget,
    progress: &dyn Fn(Progress),
) -> Result<PathBuf, RenderError> {
    let path = target.path();
    match &target.project {
        Some(project) => project.write_render(path, &out.bytes),
        None => strudeland_project::atomic_write_bytes(path, &out.bytes),
    }
    .map_err(|e| io_error(path, e))?;
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

    use crate::{Summary, Timings};
    use strudeland_host::{Layout, PlannedWindow};

    fn output(bytes: &[u8]) -> RenderOutput {
        RenderOutput {
            bytes: bytes.to_vec(),
            summary: Summary {
                format: None,
                bytes: None,
                window: PlannedWindow {
                    from_bar: 0,
                    to_bar: 1,
                    layout: Layout::Linear,
                },
                frames: 0,
                seconds: 0.0,
                sample_rate: 44_100,
                chunks: 0,
                jobs: 1,
                loudness: None,
                timings: Timings::default(),
                warnings: vec![],
            },
        }
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

    fn section(name: &str) -> LengthRequest {
        LengthRequest::Section { name: name.into() }
    }

    fn clip() -> LengthRequest {
        LengthRequest::Clip { loops: None }
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

    #[test]
    fn an_explicit_out_is_returned_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(&dir.path().join("p"), "p").unwrap();
        let song = project.root().join("songs/main.js");
        for out in ["rel/a.mp3", "/abs/b.wav"] {
            assert_eq!(
                resolve_output(Some(Path::new(out)), &song, &section("body"), Format::Ogg)
                    .unwrap()
                    .path(),
                Path::new(out)
            );
        }
        // Nothing was created for an explicit path.
        assert_eq!(
            std::fs::read_dir(project.root().join("renders"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn a_project_song_goes_to_renders() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(&dir.path().join("p"), "p").unwrap();
        let root = project.root().to_path_buf();
        let song = root.join("songs/main.js");
        let path = |length: LengthRequest, format| {
            resolve_output(None, &song, &length, format).map(|t| t.path)
        };
        let renders = root.join("renders");
        for (length, format, name) in [
            (clip(), Format::Wav, "main.wav"),
            (section("the drop!"), Format::Opus, "main-the-drop.opus"),
            // A short check or another loop count never replaces the whole song's file.
            (
                LengthRequest::Length { bars: 16 },
                Format::Wav,
                "main-len16.wav",
            ),
            (
                LengthRequest::Clip { loops: Some(2) },
                Format::Ogg,
                "main-loops2.ogg",
            ),
            (
                LengthRequest::Window {
                    from_bar: 4,
                    to_bar: 12,
                    layout: Layout::Clip,
                },
                Format::Wav,
                "main-bars4-12.wav",
            ),
        ] {
            let want = renders.join(name);
            assert_eq!(path(length.clone(), format).unwrap(), want, "{length:?}");
        }
        // A section name with nothing left after slugging is an error naming the project path.
        let e = path(section("!!"), Format::Mp3).unwrap_err();
        assert!(matches!(e, RenderError::Io { .. }), "{e:?}");
        assert_eq!(
            e.to_string(),
            format!(
                "{}: invalid render name \"!!\"",
                root.join("renders").display()
            )
        );
    }

    #[test]
    fn renders_is_created_only_when_the_file_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(&dir.path().join("p"), "p").unwrap();
        let root = project.root().to_path_buf();
        std::fs::remove_dir(root.join("renders")).unwrap();
        let song = root.join("songs/main.js");
        let target = resolve_output(None, &song, &clip(), Format::Mp3).unwrap();
        assert_eq!(target.path(), root.join("renders/main.mp3"));
        assert!(!root.join("renders").exists());
        let written = write_output(&output(b"mp3"), &target, &|_| {}).unwrap();
        assert_eq!(written, root.join("renders/main.mp3"));
        assert_eq!(std::fs::read(&written).unwrap(), b"mp3");
    }

    #[test]
    fn a_song_outside_a_project_needs_out() {
        let dir = tempfile::tempdir().unwrap();
        // No project.json.
        let songs = dir.path().join("songs");
        std::fs::create_dir(&songs).unwrap();
        let song = songs.join("a.js");
        std::fs::write(&song, "").unwrap();
        let loose = dir.path().join("b.js");
        std::fs::write(&loose, "").unwrap();
        for song in [song, loose, dir.path().join("missing.js")] {
            assert_eq!(
                usage(resolve_output(None, &song, &clip(), Format::Wav)),
                (
                    "--out",
                    format!(
                        "--out is required: {} is not in a strudeland project",
                        song.display()
                    )
                )
            );
        }
    }
}
