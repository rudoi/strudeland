//! Native audio export. Rust retains the chosen folder, checks the displayed destinations,
//! and renders the captured editor text on a blocking worker before publishing any output.

use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use strudeland_project::{
    Project, atomic_create_bytes, atomic_write_bytes, render_exists, render_filename,
};
use strudeland_render::output::{export_suffix, render_suffix};
use strudeland_render::{
    BitDepth, EncoderOptions, ExportFile, ExportPreset, ExportRole, Format, LengthRequest,
    Loudness, MAX_BARS, OutputOptions, Progress, RenderOptions, StemGrouping, StemTarget, render,
    render_export, render_stems, resolve_project_stem_output, resolve_stem_output_in,
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
        check_directory(folder)
    }
}

fn check_directory(folder: &Path) -> Result<(), ExportError> {
    if !folder.is_dir() || folder.canonicalize().map_err(problem)? != folder {
        return Err(problem(format!(
            "the selected output folder changed: {}",
            folder.display()
        )));
    }
    Ok(())
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

#[derive(Debug)]
enum Targets {
    Mix(Vec<PreviewFile>),
    Stems(StemGrouping, StemTarget),
}

impl Targets {
    fn preview(&self) -> ExportPreview {
        match self {
            Self::Mix(files) => ExportPreview::Mix(files.clone()),
            Self::Stems(_, target) => ExportPreview::Stems {
                directory: target.path().into(),
            },
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
    fn targets(&self, project: &Project, state: &ExportState) -> Result<Targets, ExportError> {
        match &self.length {
            LengthRequest::Clip { loops: None } => (),
            _ if self
                .preset
                .is_some_and(|preset| preset != ExportPreset::OneShot) =>
            {
                return Err(problem("loop exports use the marked loop section"));
            }
            LengthRequest::Section { name } if !name.is_empty() => (),
            LengthRequest::Length { bars } if (1..=MAX_BARS).contains(&u64::from(*bars)) => (),
            _ => return Err(problem("choose a valid clip, section, or length in bars")),
        }
        let song = project.resolve(&self.song_path).map_err(problem)?;
        if let Some(folder) = &self.folder {
            state.check_folder(folder)?;
        }
        if let Some(grouping) = self.stems {
            if self.preset.is_some() || self.format != Format::Wav {
                return Err(problem("stem sets require a standard range and WAV format"));
            }
            let target = match &self.folder {
                Some(folder) => resolve_stem_output_in(folder, &song, &self.length),
                None => resolve_project_stem_output(project, &song, &self.length),
            }
            .map_err(problem)?;
            target.check_available().map_err(problem)?;
            return Ok(Targets::Stems(grouping, target));
        }
        let stem = song
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| problem("invalid song filename"))?;

        let roles: &[ExportRole] = match self.preset {
            Some(ExportPreset::IntroLoop) => &[ExportRole::Intro, ExportRole::Loop],
            Some(ExportPreset::FoldedLoop) => &[ExportRole::Loop],
            _ => &[ExportRole::Single],
        };
        roles
            .iter()
            .map(|&role| {
                let suffix = match self.preset {
                    Some(preset) => {
                        Some(export_suffix(&self.length, preset, role).map_err(problem)?)
                    }
                    None => render_suffix(&self.length),
                };
                let path = match &self.folder {
                    Some(folder) => folder.join(
                        render_filename(stem, suffix.as_deref(), self.format.ext())
                            .map_err(problem)?,
                    ),
                    None => project
                        .render_path(stem, suffix.as_deref(), self.format.ext())
                        .map_err(problem)?,
                };
                let exists = render_exists(&path).map_err(problem)?;
                Ok(PreviewFile { role, path, exists })
            })
            .collect::<Result<_, _>>()
            .map(Targets::Mix)
    }

    fn options(&self) -> RenderOptions {
        RenderOptions {
            length: self.length.clone(),
            jobs: None,
            plan_timeout: Some(Duration::from_secs(120)),
            include_events: false,
            output: if self.stems.is_some() {
                OutputOptions {
                    loudness: Loudness::Off,
                    encoder: EncoderOptions::Wav {
                        bit_depth: BitDepth::F32,
                    },
                }
            } else {
                OutputOptions {
                    loudness: Loudness::default(),
                    encoder: EncoderOptions::default_for(self.format),
                }
            },
        }
    }

    fn render_mix(
        &self,
        source: &str,
        progress: &dyn Fn(Progress),
    ) -> Result<Vec<ExportFile>, ExportError> {
        let options = self.options();
        if let Some(preset) = self.preset {
            render_export(source, &options, preset, progress).map_err(problem)
        } else {
            Ok(vec![ExportFile {
                role: ExportRole::Single,
                output: render(source, &options, progress).map_err(problem)?,
                loop_points: None,
            }])
        }
    }

    fn write(
        &self,
        project: &Project,
        target: &PreviewFile,
        bytes: &[u8],
    ) -> Result<(), ExportError> {
        let path = &target.path;
        if let Some(folder) = &self.folder {
            check_directory(folder)?;
        }
        if target.exists {
            render_exists(path).map_err(problem)?;
        }
        match (&self.folder, target.exists) {
            (Some(_), true) => atomic_write_bytes(path, bytes),
            (Some(_), false) => atomic_create_bytes(path, bytes),
            (None, true) => project.write_render(path, bytes),
            (None, false) => project.create_render(path, bytes),
        }
        .map_err(problem)
    }
}

/// Resolve and compare native targets before asking for approval and again before rendering.
fn checked_targets(
    projects: &ProjectState,
    state: &ExportState,
    request: &ExportRequest,
    preview: &ExportPreview,
) -> Result<(Arc<Project>, Targets), ExportError> {
    let project = projects.current_for(&request.root).map_err(problem)?;
    let targets = request.targets(&project, state)?;
    if &targets.preview() != preview {
        return Err(problem(
            "the output files changed; review the new preview before exporting",
        ));
    }
    Ok((project, targets))
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
    let (project, targets) = checked_targets(projects, state, request, preview)?;
    let targets = match targets {
        Targets::Mix(files) => files,
        Targets::Stems(grouping, target) => {
            let written = std::cell::RefCell::new(Vec::new());
            let set = render_stems(source, &request.options(), grouping, &target, &|event| {
                if let Progress::Written { path } = &event {
                    written.borrow_mut().push(path.clone());
                }
                progress(event.into());
            })
            .map_err(|error| ExportError {
                message: error.to_string(),
                written: written.into_inner(),
            })?;
            return Ok(ExportResult::Stems {
                directory: set.directory,
                paths: set.files,
                warnings: set.summary.warnings,
            });
        }
    };
    if !replacement_approved && targets.iter().any(|file| file.exists) {
        return Err(problem("replacement requires confirmation"));
    }

    let rendered = request.render_mix(source, &|p| progress(p.into()))?;
    if !rendered
        .iter()
        .map(|f| f.role)
        .eq(targets.iter().map(|t| t.role))
    {
        return Err(problem("the rendered files do not match the preview"));
    }
    let mut files: Vec<FinishedFile> = Vec::new();
    let mut warnings = Vec::new();
    progress(ExportProgress::Writing);
    for (file, target) in rendered.into_iter().zip(&targets) {
        request
            .write(&project, target, &file.output.bytes)
            .map_err(|mut e| {
                e.written = files.iter().map(|f| f.path.clone()).collect();
                e
            })?;
        progress(ExportProgress::Written {
            path: target.path.clone(),
        });
        files.push(FinishedFile {
            role: file.role,
            path: target.path.clone(),
            godot_loop_offset: file.loop_points.map(|p| p.godot_loop_offset()),
        });
        for warning in file.output.summary.warnings {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    }
    Ok(ExportResult::Mix { files, warnings })
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
    Ok(request.targets(&project, &state)?.preview())
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
    checked_targets(
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
    use strudeland_render::{resolve_export_output, resolve_output};

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
            match self.request.targets(&self.project, &self.state).unwrap() {
                Targets::Mix(files) => files,
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
    fn names_and_roles_match_cli_for_every_format_and_preset() {
        use ExportPreset::*;
        use ExportRole::*;
        let mut f = Fixture::new();
        let song = f.project.resolve(&f.request.song_path).unwrap();
        let renders = f.project.root().join("renders");
        fs::remove_dir(&renders).unwrap();
        let section = LengthRequest::Section {
            name: "the drop!".into(),
        };
        for format in Format::ALL {
            for (preset, roles) in [
                (None, vec![Single]),
                (Some(OneShot), vec![Single]),
                (Some(FoldedLoop), vec![Loop]),
                (Some(TaggedLoop), vec![Single]),
                (Some(IntroLoop), vec![Intro, Loop]),
            ] {
                f.request.format = format;
                f.request.preset = preset;
                f.request.length = if preset.is_none() || preset == Some(OneShot) {
                    section.clone()
                } else {
                    LengthRequest::Clip { loops: None }
                };
                f.request.folder = None;
                let outputs = f.preview();
                assert_eq!(outputs.iter().map(|f| f.role).collect::<Vec<_>>(), roles);
                for target in &outputs {
                    let cli = match preset {
                        Some(preset) => resolve_export_output(
                            None,
                            &song,
                            &f.request.length,
                            format,
                            preset,
                            target.role,
                        ),
                        None => resolve_output(None, &song, &f.request.length, format),
                    }
                    .unwrap();
                    assert_eq!(target.path, cli.path());
                    assert!(!target.exists);
                }
                f.pick();
                for (custom, default) in f.preview().iter().zip(&outputs) {
                    assert_eq!(custom.path.file_name(), default.path.file_name());
                    assert_eq!(custom.path.parent(), f.request.folder.as_deref());
                    assert!(!custom.exists);
                }
            }
        }
        assert!(!renders.exists());
        fs::write(&renders, b"not a directory").unwrap();
        assert_eq!(
            f.preview().len(),
            2,
            "a picked folder is independent of project/renders"
        );
    }

    #[test]
    fn folder_song_and_project_identity_are_checked() {
        let mut f = Fixture::new();
        f.request.folder = Some(f.dir.path().canonicalize().unwrap());
        assert!(
            f.request
                .targets(&f.project, &f.state)
                .unwrap_err()
                .message
                .contains("choose")
        );
        f.pick();
        f.request.song_path = "../escape.js".into();
        assert!(f.request.targets(&f.project, &f.state).is_err());
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
        assert!(f.request.targets(&f.project, &f.state).is_err(), "preset");
        f.request.preset = None;
        f.request.format = Format::Ogg;
        assert!(
            f.request.targets(&f.project, &f.state).is_err(),
            "compressed format"
        );
        f.request.format = Format::Wav;
        // The editor still holds the buffer after an external deletion.
        fs::remove_file(f.project.resolve(&f.request.song_path).unwrap()).unwrap();
        let preview = f.request.targets(&f.project, &f.state).unwrap().preview();
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
        let picked = f.request.targets(&f.project, &f.state).unwrap().preview();
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
            let preview = f.request.targets(&f.project, &f.state).unwrap().preview();
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
}
