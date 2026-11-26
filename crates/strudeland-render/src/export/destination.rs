//! Where an export goes, and how each destination names its files.

use std::path::{Path, PathBuf};

use strudeland_project::{Project, render_filename};

use super::request::suffix;
use super::{Error, Request};
use crate::stems::{self, StemTarget};
use crate::{ExportPreset, ExportRole, RenderError};

/// Where an export goes.
#[derive(Debug)]
pub enum Destination {
    /// A given path (the CLI's `--out`): the file itself, or the stem set's directory.
    Explicit(PathBuf),
    /// A folder the user picked, canonical as picked. Files are joined to it, not re-resolved.
    ChosenFolder(PathBuf),
    /// The project's `renders/`. `stem` is the song's stem when the caller already knows it;
    /// otherwise it comes from [`Request::song`].
    Project {
        project: Project,
        stem: Option<String>,
    },
}

impl Destination {
    /// The CLI's destination: `Explicit(out)`, else the project [`Project::find`] finds for the
    /// song, with its stem, else a usage error naming `--out`.
    pub fn from_out(out: Option<&Path>, song: &Path) -> Result<Destination, Error> {
        if let Some(out) = out {
            return Ok(Destination::Explicit(out.to_owned()));
        }
        match Project::find(song) {
            Some((project, stem)) => Ok(Destination::Project {
                project,
                stem: Some(stem),
            }),
            None => Err(Error::Render(RenderError::Usage {
                flag: "--out",
                message: format!(
                    "--out is required: {} is not in a strudeland project",
                    song.display()
                ),
            })),
        }
    }

    /// The app's destination: the open project. The stem comes from [`Request::song`].
    pub fn project(project: Project) -> Destination {
        Destination::Project {
            project,
            stem: None,
        }
    }
}

/// What the files of an export will be, before anything is rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    /// One entry per role, in table order.
    Mix(Vec<PreviewFile>),
    /// The set's directory. The files are known only after planning.
    Stems { directory: PathBuf },
}

/// One mix file and whether a replaceable file is already there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFile {
    pub role: ExportRole,
    pub path: PathBuf,
    pub exists: bool,
}

/// Where a mix's files go, from the destination: how each is named and published.
#[derive(Debug)]
pub(super) enum Place {
    /// The given path, or for a split export the base of its file names.
    Explicit(PathBuf),
    ChosenFolder(PathBuf),
    Project(Project),
}

impl Place {
    /// The place of a mix and the song's stem, which names the files of a folder or project
    /// (empty for an explicit path). Derives the stem only when it is needed
    /// ([`Error::InvalidSongName`]).
    pub(super) fn for_mix(
        request: &Request,
        destination: Destination,
    ) -> Result<(Place, String), Error> {
        let song_stem = || {
            request
                .song
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
                .ok_or(Error::InvalidSongName)
        };
        Ok(match destination {
            Destination::Explicit(out) => (Place::Explicit(out), String::new()),
            Destination::ChosenFolder(folder) => (Place::ChosenFolder(folder), song_stem()?),
            Destination::Project { project, stem } => {
                let stem = match stem {
                    Some(stem) => stem,
                    None => song_stem()?,
                };
                (Place::Project(project), stem)
            }
        })
    }
}

/// Names the mix files of one place.
pub(super) struct MixNames<'a> {
    pub(super) request: &'a Request,
    pub(super) preset: Option<ExportPreset>,
    pub(super) ext: &'static str,
    pub(super) place: &'a Place,
    pub(super) stem: &'a str,
}

impl MixNames<'_> {
    /// The path of one role's file. Creates nothing.
    pub(super) fn path(&self, role: ExportRole) -> Result<PathBuf, Error> {
        let suffix = suffix(&self.request.length, self.preset, role)?;
        let stem = self.stem;
        match self.place {
            Place::Explicit(out) => match (self.preset, suffix) {
                (Some(ExportPreset::IntroLoop), Some(suffix)) => {
                    let mut name = out
                        .file_stem()
                        .ok_or_else(|| {
                            Error::Render(RenderError::Usage {
                                flag: "--out",
                                message: "the split export needs a file name for --out".into(),
                            })
                        })?
                        .to_os_string();
                    name.push(format!("-{suffix}.{}", self.ext));
                    Ok(out.with_file_name(name))
                }
                _ => Ok(out.clone()),
            },
            Place::ChosenFolder(folder) => render_filename(stem, suffix.as_deref(), self.ext)
                .map(|name| folder.join(name))
                .map_err(|source| Error::Project {
                    at: folder.clone(),
                    source,
                }),
            Place::Project(project) => project
                .render_path(stem, suffix.as_deref(), self.ext)
                .map_err(|source| Error::Project {
                    at: project.root().join("renders"),
                    source,
                }),
        }
    }
}

/// Resolves a stem set's directory. Checks nothing on disk beyond what the resolvers do.
pub(super) fn stem_target(
    request: &Request,
    destination: Destination,
) -> Result<StemTarget, Error> {
    let (song, length) = (&request.song, &request.length);
    match destination {
        Destination::Explicit(out) => stems::explicit_target(&out),
        Destination::ChosenFolder(folder) => stems::resolve_stem_output_in(&folder, song, length),
        Destination::Project {
            project,
            stem: Some(stem),
        } => stems::project_target(&project, &stem, length),
        Destination::Project {
            project,
            stem: None,
        } => stems::resolve_project_stem_output(&project, song, length),
    }
    .map_err(Error::Render)
}

#[cfg(test)]
mod tests {
    use super::super::{Kind, prepare, prepare_previewed};
    use super::*;
    use crate::{EncoderOptions, Format, LengthRequest, Loudness, OutputOptions};
    use strudeland_host::Layout;

    fn mix(
        song: &Path,
        length: LengthRequest,
        preset: Option<ExportPreset>,
        format: Format,
    ) -> Request {
        Request {
            song: song.to_owned(),
            length,
            kind: Kind::Mix {
                preset,
                output: OutputOptions {
                    loudness: Loudness::default(),
                    encoder: EncoderOptions::default_for(format),
                },
            },
            jobs: None,
            plan_timeout: None,
        }
    }

    fn section(name: &str) -> LengthRequest {
        LengthRequest::Section { name: name.into() }
    }

    const CLIP: LengthRequest = LengthRequest::Clip { loops: None };

    fn usage(r: Result<Destination, Error>) -> (&'static str, String) {
        match r {
            Err(Error::Render(RenderError::Usage { flag, message })) => (flag, message),
            other => panic!("not a usage error: {other:?}"),
        }
    }

    #[test]
    fn destinations_resolve_every_role_without_creating_anything() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(&dir.path().join("p"), "p").unwrap();
        let root = project.root().to_path_buf();
        let renders = root.join("renders");
        std::fs::remove_dir(&renders).unwrap();
        let song = root.join("songs/main.js");
        let out_dir = dir.path().join("out");
        std::fs::create_dir(&out_dir).unwrap();
        let folder = out_dir.canonicalize().unwrap();
        let presets: [(Option<ExportPreset>, &[&str]); 5] = [
            (None, &["main-the-drop"]),
            // Today's slug: "the drop!-one-shot" keeps the `-` after the `!` run's `-`.
            (Some(ExportPreset::OneShot), &["main-the-drop--one-shot"]),
            (Some(ExportPreset::TaggedLoop), &["main-tagged-loop"]),
            (Some(ExportPreset::FoldedLoop), &["main-folded-loop"]),
            (Some(ExportPreset::IntroLoop), &["main-intro", "main-loop"]),
        ];
        for (preset, names) in presets {
            let length = match preset {
                None | Some(ExportPreset::OneShot) => section("the drop!"),
                _ => CLIP,
            };
            for format in Format::ALL {
                let ext = format.ext();
                let request = mix(&song, length.clone(), preset, format);
                let explicit: Vec<PathBuf> = if preset == Some(ExportPreset::IntroLoop) {
                    vec![
                        folder.join(format!("pair-intro.{ext}")),
                        folder.join(format!("pair-loop.{ext}")),
                    ]
                } else {
                    vec![folder.join("pair.wav")]
                };
                let named = |dir: &Path| -> Vec<PathBuf> {
                    names
                        .iter()
                        .map(|n| dir.join(format!("{n}.{ext}")))
                        .collect()
                };
                let destination = |i: usize| match i {
                    0 => Destination::Explicit(folder.join("pair.wav")),
                    1 => Destination::ChosenFolder(folder.clone()),
                    2 => Destination::from_out(None, &song).unwrap(),
                    _ => Destination::project(project.clone()),
                };
                let cases = [
                    (0, explicit),
                    (1, named(&folder)),
                    (2, named(&renders)),
                    (3, named(&renders)),
                ];
                for (i, want) in cases {
                    let label = format!("{preset:?} {format:?} {want:?}");
                    let prepared = prepare(request.clone(), destination(i)).unwrap();
                    let got: Vec<_> = prepared
                        .targets()
                        .iter()
                        .map(|t| (t.role, t.path.clone()))
                        .collect();
                    let expected: Vec<_> = roles_with(preset, &want);
                    assert_eq!(got, expected, "{label}");
                    assert_eq!(prepared.preset(), preset);
                    assert_eq!(prepared.stem_directory(), None);
                    let (previewed, preview) =
                        prepare_previewed(request.clone(), destination(i)).unwrap();
                    assert_eq!(previewed.targets(), prepared.targets(), "{label}");
                    assert_eq!(
                        preview,
                        Preview::Mix(
                            expected
                                .into_iter()
                                .map(|(role, path)| PreviewFile {
                                    role,
                                    path,
                                    exists: false
                                })
                                .collect()
                        ),
                        "{label}"
                    );
                }
            }
        }
        assert!(!renders.exists());
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);

        // A chosen folder never touches the project's renders/.
        std::fs::write(&renders, b"not a directory").unwrap();
        let request = mix(&song, CLIP, Some(ExportPreset::IntroLoop), Format::Wav);
        let (_, preview) =
            prepare_previewed(request, Destination::ChosenFolder(folder.clone())).unwrap();
        assert!(matches!(&preview, Preview::Mix(files) if files.len() == 2));
    }

    fn roles_with(preset: Option<ExportPreset>, paths: &[PathBuf]) -> Vec<(ExportRole, PathBuf)> {
        crate::export::roles(preset)
            .iter()
            .copied()
            .zip(paths.iter().cloned())
            .collect()
    }

    #[test]
    fn resolver_names_and_errors_match_the_cli() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(&dir.path().join("p"), "p").unwrap();
        let root = project.root().to_path_buf();
        let song = root.join("songs/main.js");
        let renders = root.join("renders");
        let path = |length: LengthRequest, format| {
            prepare(
                mix(&song, length, None, format),
                Destination::from_out(None, &song).unwrap(),
            )
            .map(|p| {
                p.targets()
                    .iter()
                    .map(|t| t.path.clone())
                    .collect::<Vec<_>>()
            })
        };
        for (length, format, name) in [
            (CLIP, Format::Wav, "main.wav"),
            (section("the drop!"), Format::Opus, "main-the-drop.opus"),
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
            assert_eq!(
                path(length.clone(), format).unwrap(),
                [renders.join(name)],
                "{length:?}"
            );
        }
        // A section name with nothing left after slugging names the project's renders/.
        let e = RenderError::from(path(section("!!"), Format::Mp3).unwrap_err());
        assert!(matches!(e, RenderError::Io { .. }), "{e:?}");
        assert_eq!(
            e.to_string(),
            format!("{}: invalid render name \"!!\"", renders.display())
        );

        // A song outside a project needs --out.
        let outside = tempfile::tempdir().unwrap();
        let songs = outside.path().join("songs");
        std::fs::create_dir(&songs).unwrap();
        let in_songs = songs.join("a.js");
        std::fs::write(&in_songs, "").unwrap();
        let loose = outside.path().join("b.js");
        std::fs::write(&loose, "").unwrap();
        for song in [in_songs, loose, outside.path().join("missing.js")] {
            assert_eq!(
                usage(Destination::from_out(None, &song)),
                (
                    "--out",
                    format!(
                        "--out is required: {} is not in a strudeland project",
                        song.display()
                    )
                )
            );
        }

        // An explicit --out is returned unchanged.
        for out in ["rel/a.mp3", "/abs/b.wav"] {
            let prepared = prepare(
                mix(&song, section("body"), None, Format::Ogg),
                Destination::from_out(Some(Path::new(out)), &song).unwrap(),
            )
            .unwrap();
            assert_eq!(prepared.targets()[0].path, Path::new(out));
        }
        assert_eq!(std::fs::read_dir(&renders).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn each_entry_point_keeps_its_adapters_error_order() {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(&dir.path().join("p"), "p").unwrap();
        let root = project.root().to_path_buf();
        let song = root.join("songs/main.js");
        for name in ["main-intro.wav", "main-loop.wav"] {
            let outside = dir.path().join(name);
            std::fs::write(&outside, b"x").unwrap();
            std::os::unix::fs::symlink(&outside, root.join("renders").join(name)).unwrap();
        }
        let request = mix(&song, CLIP, Some(ExportPreset::IntroLoop), Format::Wav);
        let source = |e: Error| match e {
            Error::Project { source, .. } => source.to_string(),
            other => panic!("{other:?}"),
        };
        let cli =
            prepare(request.clone(), Destination::from_out(None, &song).unwrap()).unwrap_err();
        assert!(source(cli).contains("renders/main-loop.wav"));
        let app = prepare_previewed(request, Destination::project(project.clone())).unwrap_err();
        assert!(source(app).contains("renders/main-intro.wav"));
    }
}
