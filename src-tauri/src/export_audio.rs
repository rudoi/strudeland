//! Native audio export. Rust retains the chosen folder, checks the displayed destinations,
//! and renders the captured editor text on a blocking worker before publishing any output.

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use strudeland_project::Project;
use strudeland_render::export;
use strudeland_render::{
    EncoderOptions, ExportPreset, ExportRole, Format, LengthRequest, Loudness, MAX_BARS,
    OutputOptions, Progress, StemGrouping,
};
use tauri::{Manager, State, WebviewWindow, ipc::Channel};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

use crate::project::ProjectState;

#[derive(Default)]
pub struct ExportState {
    folder: Mutex<Option<PathBuf>>,
    running: Arc<tokio::sync::Mutex<()>>,
}

impl ExportState {
    pub(super) fn reserve(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, ExportError> {
        Arc::clone(&self.running)
            .try_lock_owned()
            .map_err(|_| problem("an audio export is already running"))
    }

    fn check_folder(&self, folder: &Path) -> Result<(), ExportError> {
        if self
            .folder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
            != Some(folder)
        {
            return Err(problem("choose the output folder again"));
        }
        export::check_chosen_folder(folder).map_err(export_error)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportRequest {
    root: String,
    song_path: String,
    folder: Option<PathBuf>,
    format: Format,
    preset: Option<ExportPreset>,
    stems: Option<StemGrouping>,
    length: LengthRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewFile {
    role: ExportRole,
    path: PathBuf,
    exists: bool,
}

/// A mix previews its individual paths. Stems are discovered during planning, so only
/// their common directory is available when the dialog opens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ExportPreview {
    Mix(Vec<PreviewFile>),
    Stems { directory: PathBuf },
}

impl From<export::Preview> for ExportPreview {
    fn from(preview: export::Preview) -> Self {
        match preview {
            export::Preview::Mix(files) => Self::Mix(
                files
                    .into_iter()
                    .map(|file| PreviewFile {
                        role: file.role,
                        path: file.path,
                        exists: file.exists,
                    })
                    .collect(),
            ),
            export::Preview::Stems { directory } => Self::Stems { directory },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishedFile {
    role: ExportRole,
    path: PathBuf,
    godot_loop_offset: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ExportResult {
    Mix {
        files: Vec<FinishedFile>,
        warnings: Vec<String>,
    },
    Stems {
        directory: PathBuf,
        paths: Vec<PathBuf>,
        warnings: Vec<String>,
    },
}

#[derive(Debug, Serialize)]
pub struct ExportError {
    message: String,
    written: Vec<PathBuf>,
}

fn problem(error: impl Display) -> ExportError {
    ExportError {
        message: error.to_string(),
        written: Vec::new(),
    }
}

/// The dialog's wording of an export module error.
fn export_message(error: &export::Error) -> String {
    match error {
        export::Error::Render(e) => e.to_string(),
        export::Error::Project { source, .. } => source.to_string(),
        export::Error::FolderChanged { path } => {
            format!("the selected output folder changed: {}", path.display())
        }
        export::Error::FolderUnreadable { source, .. } => source.to_string(),
        export::Error::InvalidSongName => "invalid song filename".into(),
        export::Error::RolesMismatch { .. } => "the rendered files do not match the preview".into(),
        export::Error::PreviewMismatch => {
            "the output files changed; review the new preview before exporting".into()
        }
        export::Error::ReplacementNotApproved => "replacement requires confirmation".into(),
        export::Error::PreviewRequired => "the export has no preview".into(),
    }
}

fn export_error(error: export::Error) -> ExportError {
    ExportError {
        message: export_message(&error),
        written: Vec::new(),
    }
}

/// A failed run, with the files already published when it stopped part way.
fn run_error(error: export::RunError) -> ExportError {
    ExportError {
        message: export_message(&error.source),
        written: error.written.into_iter().map(|w| w.path).collect(),
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ExportProgress {
    Confirming,
    Planning,
    Planned { chunks: usize },
    ChunkRendered { index: usize, of: usize },
    Encoded { pass: u32 },
    Writing,
    Written { path: PathBuf },
}

impl From<Progress> for ExportProgress {
    fn from(event: Progress) -> Self {
        match event {
            Progress::Planned { chunks } => Self::Planned { chunks },
            Progress::ChunkRendered { index, of } => Self::ChunkRendered { index, of },
            Progress::Encoded { pass } => Self::Encoded { pass },
            Progress::Written { path } => Self::Written { path },
        }
    }
}

impl ExportRequest {
    /// The checks the dialog shows, in the app's order, then the module's resolution and
    /// preview. Creates nothing.
    fn prepare(
        &self,
        project: &Project,
        state: &ExportState,
    ) -> Result<(export::Prepared, export::Preview), ExportError> {
        export::check_length_for_preset(&self.length, self.preset)
            .map_err(|_| problem("loop exports use the marked loop section"))?;
        match &self.length {
            LengthRequest::Clip { loops: None } => (),
            LengthRequest::Section { name } if !name.is_empty() => (),
            LengthRequest::Length { bars } if (1..=MAX_BARS).contains(&u64::from(*bars)) => (),
            _ => return Err(problem("choose a valid clip, section, or length in bars")),
        }
        let song = project.resolve(&self.song_path).map_err(problem)?;
        if let Some(folder) = &self.folder {
            state.check_folder(folder)?;
        }
        let kind = match self.stems {
            Some(grouping) => {
                if self.preset.is_some() || self.format != Format::Wav {
                    return Err(problem("stem sets require a standard range and WAV format"));
                }
                export::Kind::Stems { grouping }
            }
            None => export::Kind::Mix {
                preset: self.preset,
                output: OutputOptions {
                    loudness: Loudness::default(),
                    encoder: EncoderOptions::default_for(self.format),
                },
            },
        };
        let destination = match &self.folder {
            Some(folder) => export::Destination::ChosenFolder(folder.clone()),
            None => export::Destination::project(project.clone()),
        };
        export::prepare_previewed(
            export::Request {
                song,
                length: self.length.clone(),
                kind,
                jobs: None,
                plan_timeout: Some(Duration::from_secs(120)),
            },
            destination,
        )
        .map_err(export_error)
    }
}

/// Resolve and compare native targets before asking for approval and again before rendering.
fn checked(
    projects: &ProjectState,
    state: &ExportState,
    request: &ExportRequest,
    preview: &ExportPreview,
) -> Result<(Arc<Project>, export::Prepared, export::Preview), ExportError> {
    let project = projects.current_for(&request.root).map_err(problem)?;
    let (prepared, fresh) = request.prepare(&project, state)?;
    if ExportPreview::from(fresh.clone()) != *preview {
        return Err(problem(
            "the output files changed; review the new preview before exporting",
        ));
    }
    Ok((project, prepared, fresh))
}

fn run(
    projects: &ProjectState,
    state: &ExportState,
    request: &ExportRequest,
    source: &str,
    preview: &ExportPreview,
    replacement_approved: bool,
    progress: &dyn Fn(ExportProgress),
) -> Result<ExportResult, ExportError> {
    let (project, prepared, fresh) = checked(projects, state, request, preview)?;
    let instruments = project.instrument_snapshot().map_err(problem)?;
    let finished = prepared
        .run(
            &mut export::RealPipeline,
            source,
            export::ReplacePolicy::CreateNew {
                approved: replacement_approved,
            },
            Some(&fresh),
            instruments,
            &|event| match event {
                export::Event::Pipeline(p) => progress(p.into()),
                export::Event::Publishing => progress(ExportProgress::Writing),
                export::Event::Written { file, .. } => progress(ExportProgress::Written {
                    path: file.path.clone(),
                }),
            },
        )
        .map_err(run_error)?;
    Ok(match finished {
        export::Finished::Mix {
            files, warnings, ..
        } => ExportResult::Mix {
            files: files
                .into_iter()
                .map(|file| FinishedFile {
                    role: file.role,
                    path: file.path,
                    godot_loop_offset: file.loop_points.map(|p| p.godot_loop_offset()),
                })
                .collect(),
            warnings,
        },
        export::Finished::Stems { export, .. } => ExportResult::Stems {
            directory: export.directory,
            paths: export.files,
            warnings: export.summary.warnings,
        },
    })
}

#[tauri::command]
pub async fn export_audio_pick_folder(
    window: WebviewWindow,
    state: State<'_, ExportState>,
) -> Result<Option<PathBuf>, ExportError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = window.dialog().file();
    #[cfg(any(target_os = "macos", windows))]
    let dialog = dialog.set_parent(&window);
    dialog
        .set_title("Choose export folder")
        .pick_folder(move |folder| {
            let _ = tx.send(folder);
        });
    let Some(folder) = rx.await.map_err(problem)? else {
        return Ok(None);
    };
    let folder = folder
        .into_path()
        .map_err(problem)?
        .canonicalize()
        .map_err(problem)?;
    if folder.to_str().is_none() {
        return Err(problem("the selected folder name cannot be displayed"));
    }
    *state.folder.lock().unwrap_or_else(|e| e.into_inner()) = Some(folder.clone());
    Ok(Some(folder))
}

#[tauri::command]
pub async fn export_audio_preview(
    projects: State<'_, ProjectState>,
    state: State<'_, ExportState>,
    request: ExportRequest,
) -> Result<ExportPreview, ExportError> {
    let project = projects.current_for(&request.root).map_err(problem)?;
    request
        .prepare(&project, &state)
        .map(|(_, preview)| preview.into())
}

const REPLACE: &str = "Replace";

async fn confirm_replacement(
    window: &WebviewWindow,
    paths: &[String],
) -> Result<bool, ExportError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let dialog = window.dialog().message(format!(
        "Replace the existing audio files at these paths?\n\n{}",
        paths.join("\n")
    ));
    #[cfg(any(target_os = "macos", windows))]
    let dialog = dialog.parent(window);
    dialog
        .title("Replace audio files?")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            REPLACE.into(),
            "Cancel".into(),
        ))
        .show_with_result(move |choice| {
            let _ =
                tx.send(matches!(choice, MessageDialogResult::Custom(label) if label == REPLACE));
        });
    rx.await.map_err(problem)
}

#[tauri::command]
pub async fn export_audio_start(
    window: WebviewWindow,
    request: ExportRequest,
    source: String,
    preview: ExportPreview,
    on_progress: Channel<ExportProgress>,
) -> Result<Option<ExportResult>, ExportError> {
    let permit = window.state::<ExportState>().reserve()?;
    checked(
        &window.state::<ProjectState>(),
        &window.state::<ExportState>(),
        &request,
        &preview,
    )?;
    let existing: Vec<_> = match &preview {
        ExportPreview::Mix(files) => files
            .iter()
            .filter(|file| file.exists)
            .map(|file| file.path.display().to_string())
            .collect(),
        ExportPreview::Stems { .. } => Vec::new(),
    };
    let mut replacement_approved = false;
    if !existing.is_empty() {
        let _ = on_progress.send(ExportProgress::Confirming);
        replacement_approved = confirm_replacement(&window, &existing).await?;
        if !replacement_approved {
            return Ok(None);
        }
    }
    let _ = on_progress.send(ExportProgress::Planning);
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        run(
            &window.state::<ProjectState>(),
            &window.state::<ExportState>(),
            &request,
            &source,
            &preview,
            replacement_approved,
            &|event| {
                let _ = on_progress.send(event);
            },
        )
        .map(Some)
    })
    .await
    .map_err(problem)?
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;

    use super::*;
    use strudeland_project::read_wav_header;

    struct Fixture {
        dir: tempfile::TempDir,
        projects: ProjectState,
        project: Arc<Project>,
        state: ExportState,
        request: ExportRequest,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let project = Project::create(&dir.path().join("project"), "project").unwrap();
            let projects = ProjectState::default();
            let root = projects.open_unwatched(project);
            let project = projects.current_for(&root).unwrap();
            let request = ExportRequest {
                root,
                song_path: "songs/main.js".into(),
                format: Format::Wav,
                folder: None,
                preset: Some(ExportPreset::IntroLoop),
                stems: None,
                length: LengthRequest::Clip { loops: None },
            };
            Self {
                dir,
                projects,
                project,
                state: ExportState::default(),
                request,
            }
        }

        fn preview(&self) -> Vec<PreviewFile> {
            match ExportPreview::from(self.request.prepare(&self.project, &self.state).unwrap().1) {
                ExportPreview::Mix(files) => files,
                _ => panic!("expected mix files"),
            }
        }

        fn run(
            &self,
            source: &str,
            preview: &[PreviewFile],
            replacement_approved: bool,
            progress: &dyn Fn(ExportProgress),
        ) -> Result<ExportResult, ExportError> {
            run(
                &self.projects,
                &self.state,
                &self.request,
                source,
                &ExportPreview::Mix(preview.to_owned()),
                replacement_approved,
                progress,
            )
        }

        fn pick(&mut self) {
            let path = self.dir.path().canonicalize().unwrap();
            *self.state.folder.lock().unwrap() = Some(path.clone());
            self.request.folder = Some(path);
        }
    }

    #[test]
    fn folder_song_and_project_identity_are_checked() {
        let mut f = Fixture::new();
        f.request.folder = Some(f.dir.path().canonicalize().unwrap());
        assert!(
            f.request
                .prepare(&f.project, &f.state)
                .unwrap_err()
                .message
                .contains("choose")
        );
        f.pick();
        f.request.song_path = "../escape.js".into();
        assert!(f.request.prepare(&f.project, &f.state).is_err());
        f.request.song_path = "songs/main.js".into();
        let preview = f.preview();
        f.projects
            .open_unwatched(Project::create(&f.dir.path().join("other"), "other").unwrap());
        assert_eq!(
            f.run("unreachable", &preview, false, &|_| panic!())
                .unwrap_err()
                .message,
            crate::project::PROJECT_CHANGED
        );
    }

    #[test]
    fn reserve_one_job_and_release_on_drop() {
        let state = ExportState::default();
        let permit = state.reserve().unwrap();
        assert!(state.reserve().is_err());
        drop(permit);
        assert!(state.reserve().is_ok());
    }

    #[test]
    fn a_changed_preview_is_refused_before_planning() {
        let f = Fixture::new();
        let preview = f.preview();
        fs::write(&preview[1].path, b"existing repeat").unwrap();
        let e = f
            .run("unreachable", &preview, false, &|_| panic!())
            .unwrap_err();
        assert!(e.message.contains("review the new preview"));
        assert!(e.written.is_empty());
        assert!(!preview[0].path.exists());
        assert!(f.preview()[1].exists);
    }

    #[test]
    fn matching_preview_cannot_authorize_overwriting_a_file() {
        let f = Fixture::new();
        let opening = f.preview()[0].path.clone();
        fs::write(&opening, b"original audio").unwrap();
        let preview = f.preview();
        assert!(preview[0].exists);
        let err = f
            .run("unreachable", &preview, false, &|_| panic!())
            .unwrap_err();
        assert!(err.message.contains("confirmation"));
        assert!(err.written.is_empty());
        assert_eq!(fs::read(&opening).unwrap(), b"original audio");
        assert!(!preview[1].path.exists());
    }

    const BODY: &str = "const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'body', bars: 1, loop: true }] };\nsong(meta, { body: note('a4').s('sine') })";
    const INTRO_BODY: &str = "const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'intro', bars: 1 }, { name: 'body', bars: 1, loop: true }] };\nsong(meta, { intro: note('a3').s('sine'), body: note('a4').s('sine') })";

    #[test]
    fn stem_preview_rejects_incompatible_settings_and_occupied_destinations() {
        let mut f = Fixture::new();
        f.request.stems = Some(StemGrouping::Voice);
        assert!(f.request.prepare(&f.project, &f.state).is_err(), "preset");
        f.request.preset = None;
        f.request.format = Format::Ogg;
        assert!(
            f.request.prepare(&f.project, &f.state).is_err(),
            "compressed format"
        );
        f.request.format = Format::Wav;
        // The editor still holds the buffer after an external deletion.
        fs::remove_file(f.project.resolve(&f.request.song_path).unwrap()).unwrap();
        let preview = ExportPreview::from(f.request.prepare(&f.project, &f.state).unwrap().1);
        let ExportPreview::Stems { directory } = &preview else {
            panic!("expected stem set")
        };
        assert_eq!(directory, &f.project.root().join("renders/main-stems"));
        assert!(!directory.exists());
        fs::create_dir(directory).unwrap();
        let marker = directory.join("keep");
        fs::write(&marker, "original").unwrap();
        assert!(
            run(
                &f.projects,
                &f.state,
                &f.request,
                "unreachable",
                &preview,
                true,
                &|_| panic!()
            )
            .is_err()
        );
        assert_eq!(fs::read(marker).unwrap(), b"original");
        f.pick();
        let picked = ExportPreview::from(f.request.prepare(&f.project, &f.state).unwrap().1);
        assert_eq!(
            picked,
            ExportPreview::Stems {
                directory: f.request.folder.unwrap().join("main-stems")
            }
        );
    }

    #[test]
    fn stem_export_lists_written_paths_and_reports_collisions_after_publication() {
        let mut f = Fixture::new();
        f.request.preset = None;
        f.request.stems = Some(StemGrouping::Orbit);
        for collision in [false, true] {
            if collision {
                f.pick();
            }
            let preview = ExportPreview::from(f.request.prepare(&f.project, &f.state).unwrap().1);
            let ExportPreview::Stems { directory } = &preview else {
                panic!("expected stem set")
            };
            let manifest = directory.join("stems.json");
            let published = RefCell::new(Vec::new());
            let result = run(
                &f.projects,
                &f.state,
                &f.request,
                BODY,
                &preview,
                false,
                &|event| {
                    if let ExportProgress::Written { path } = event {
                        published.borrow_mut().push(path);
                        if collision && published.borrow().len() == 1 {
                            fs::write(&manifest, b"competing file").unwrap();
                        }
                    }
                },
            );
            let published = published.into_inner();
            if collision {
                let error = result.unwrap_err();
                assert!(!published.is_empty());
                assert_eq!(error.written, published);
                assert_eq!(fs::read(&manifest).unwrap(), b"competing file");
            } else {
                let ExportResult::Stems {
                    directory: finished,
                    paths,
                    ..
                } = result.unwrap()
                else {
                    panic!("expected stems")
                };
                assert_eq!(finished, *directory);
                assert_eq!(paths, published);
                assert_eq!(paths.last(), Some(&manifest));
                assert!(paths.contains(&directory.join("mix.wav")));
                assert!(paths.len() >= 3);
                assert!(paths.iter().all(|file| file.is_file()));
            }
        }
    }

    #[test]
    fn split_wavs_include_the_first_pass_and_encoded_loop_markers() {
        let f = Fixture::new();
        for (source, opening_frames, approve_replacement) in
            [(INTRO_BODY, 22_050, false), (BODY, 11_025, true)]
        {
            let preview = f.preview();
            let events = RefCell::new(Vec::new());
            let result = f
                .run(source, &preview, approve_replacement, &|event| {
                    events.borrow_mut().push(event)
                })
                .unwrap();
            let ExportResult::Mix { files, .. } = result else {
                panic!("expected mix result")
            };
            let [opening, repeat] = &files[..] else {
                panic!("expected two files");
            };
            assert_eq!(opening.role, ExportRole::Intro);
            assert_eq!(opening.godot_loop_offset, None);
            assert_eq!(repeat.role, ExportRole::Loop);
            assert_eq!(repeat.godot_loop_offset, Some(0.0));
            assert_eq!(
                read_wav_header(&opening.path).unwrap().frames,
                opening_frames
            );
            assert_eq!(read_wav_header(&repeat.path).unwrap().frames, 11_025);
            // The encoder appends a 60-byte sampler chunk after the audio data.
            let bytes = fs::read(&repeat.path).unwrap();
            let marker = &bytes[bytes.len() - 68..];
            assert_eq!(&marker[..4], b"smpl");
            assert_eq!(u32::from_le_bytes(marker[52..56].try_into().unwrap()), 0);
            assert_eq!(
                u32::from_le_bytes(marker[56..60].try_into().unwrap()),
                11_024
            );
            let events = events.into_inner();
            assert!(matches!(
                events.first(),
                Some(ExportProgress::Planned { .. })
            ));
            assert!(
                events
                    .iter()
                    .any(|p| matches!(p, ExportProgress::ChunkRendered { .. }))
            );
            assert!(
                events
                    .iter()
                    .any(|p| matches!(p, ExportProgress::Encoded { .. }))
            );
            assert!(matches!(
                &events[events.len() - 3..],
                [
                    ExportProgress::Writing,
                    ExportProgress::Written { .. },
                    ExportProgress::Written { .. }
                ]
            ));
        }
    }

    #[test]
    fn a_collision_during_publish_preserves_the_new_file_and_reports_partial_output() {
        let mut f = Fixture::new();
        for custom in [false, true] {
            if custom {
                f.pick();
            }
            let preview = f.preview();
            let collision = &preview[1].path;
            let e = f
                .run(BODY, &preview, false, &|p| {
                    if matches!(p, ExportProgress::Written { .. }) {
                        fs::write(collision, b"concurrent file").unwrap();
                    }
                })
                .unwrap_err();
            assert_eq!(e.written, [preview[0].path.clone()]);
            assert!(read_wav_header(&preview[0].path).is_some());
            assert_eq!(fs::read(collision).unwrap(), b"concurrent file");
        }
    }

    #[test]
    fn export_messages_keep_their_wording() {
        use std::io;
        use strudeland_project::ProjectError;
        use strudeland_render::RenderError;
        let outside = ProjectError::PathOutsideProject("renders/x.wav".into());
        let outside_text = outside.to_string();
        let cases: Vec<(export::Error, &str)> = vec![
            (
                export::Error::Render(RenderError::Encode("boom".into())),
                "encode failed: boom",
            ),
            (
                export::Error::Project {
                    at: "/p/renders".into(),
                    source: outside,
                },
                &outside_text,
            ),
            (
                export::Error::FolderChanged {
                    path: "/out".into(),
                },
                "the selected output folder changed: /out",
            ),
            (
                export::Error::FolderUnreadable {
                    path: "/out".into(),
                    source: io::Error::other("gone"),
                },
                "gone",
            ),
            (export::Error::InvalidSongName, "invalid song filename"),
            (
                export::Error::RolesMismatch { extra_intro: false },
                "the rendered files do not match the preview",
            ),
            (
                export::Error::PreviewMismatch,
                "the output files changed; review the new preview before exporting",
            ),
            (
                export::Error::ReplacementNotApproved,
                "replacement requires confirmation",
            ),
            (export::Error::PreviewRequired, "the export has no preview"),
        ];
        for (error, text) in cases {
            let label = format!("{error:?}");
            let e = run_error(error.into());
            assert_eq!(e.message, text, "{label}");
            assert!(e.written.is_empty(), "{label}");
        }
        let e = run_error(export::RunError {
            written: vec![export::WrittenPath {
                path: "renders/main-intro.wav".into(),
                absolute: "/abs/renders/main-intro.wav".into(),
            }],
            failed: Some("renders/main-loop.wav".into()),
            source: export::Error::RolesMismatch { extra_intro: true },
        });
        assert_eq!(e.message, "the rendered files do not match the preview");
        assert_eq!(e.written, [PathBuf::from("renders/main-intro.wav")]);
    }

    #[cfg(unix)]
    #[test]
    fn preview_order_is_the_apps() {
        let f = Fixture::new();
        let renders = f.project.root().join("renders");
        let intro = renders.join("main-intro.wav");
        fs::create_dir(&intro).unwrap();
        let outside = f.dir.path().join("elsewhere.wav");
        fs::write(&outside, b"x").unwrap();
        std::os::unix::fs::symlink(&outside, renders.join("main-loop.wav")).unwrap();
        let e = f.request.prepare(&f.project, &f.state).unwrap_err();
        assert_eq!(
            e.message,
            format!("{}: not a regular output file", intro.display())
        );
        assert!(e.written.is_empty());
    }
}
