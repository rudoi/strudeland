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
    EncoderOptions, ExportFile, ExportPreset, ExportRole, Format, LengthRequest, Loudness,
    MAX_BARS, OutputOptions, Progress, RenderOptions, render, render_export,
};
use tauri::{Manager, State, WebviewWindow, ipc::Channel};
use tauri_plugin_dialog::DialogExt;

use crate::project::ProjectState;

#[derive(Default)]
pub struct ExportState {
    folder: Mutex<Option<PathBuf>>,
    running: Arc<tokio::sync::Mutex<()>>,
}

impl ExportState {
    fn reserve(&self) -> Result<tokio::sync::OwnedMutexGuard<()>, ExportError> {
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
    length: LengthRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewFile {
    role: ExportRole,
    path: PathBuf,
    exists: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishedFile {
    role: ExportRole,
    path: PathBuf,
    godot_loop_offset: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    files: Vec<FinishedFile>,
    warnings: Vec<String>,
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
    fn targets(
        &self,
        project: &Project,
        state: &ExportState,
    ) -> Result<Vec<PreviewFile>, ExportError> {
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
        let stem = song
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| problem("invalid song filename"))?;
        if let Some(folder) = &self.folder {
            state.check_folder(folder)?;
        }

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
            .collect()
    }

    fn render(
        &self,
        source: &str,
        progress: &dyn Fn(Progress),
    ) -> Result<Vec<ExportFile>, ExportError> {
        let options = RenderOptions {
            length: self.length.clone(),
            jobs: None,
            plan_timeout: Some(Duration::from_secs(120)),
            include_events: false,
            output: OutputOptions {
                loudness: Loudness::default(),
                encoder: EncoderOptions::default_for(self.format),
            },
        };
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

fn run(
    projects: &ProjectState,
    state: &ExportState,
    request: &ExportRequest,
    source: &str,
    preview: &[PreviewFile],
    progress: &dyn Fn(ExportProgress),
) -> Result<ExportResult, ExportError> {
    let project = projects.current_for(&request.root).map_err(problem)?;
    let targets = request.targets(&project, state)?;
    if targets != preview {
        return Err(problem(
            "the output files changed; review the new preview before exporting",
        ));
    }

    let rendered = request.render(source, &|p| progress(p.into()))?;
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
    Ok(ExportResult { files, warnings })
}

#[tauri::command]
pub async fn export_audio_pick_folder(
    window: WebviewWindow,
    state: State<'_, ExportState>,
) -> Result<Option<PathBuf>, ExportError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    window
        .dialog()
        .file()
        .set_parent(&window)
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
) -> Result<Vec<PreviewFile>, ExportError> {
    let project = projects.current_for(&request.root).map_err(problem)?;
    request.targets(&project, &state)
}

#[tauri::command]
pub async fn export_audio_start(
    app: tauri::AppHandle,
    request: ExportRequest,
    source: String,
    preview: Vec<PreviewFile>,
    on_progress: Channel<ExportProgress>,
) -> Result<ExportResult, ExportError> {
    let permit = app.state::<ExportState>().reserve()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        run(
            &app.state::<ProjectState>(),
            &app.state::<ExportState>(),
            &request,
            &source,
            &preview,
            &|event| {
                let _ = on_progress.send(event);
            },
        )
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
            self.request.targets(&self.project, &self.state).unwrap()
        }

        fn run(
            &self,
            source: &str,
            preview: &[PreviewFile],
            progress: &dyn Fn(ExportProgress),
        ) -> Result<ExportResult, ExportError> {
            run(
                &self.projects,
                &self.state,
                &self.request,
                source,
                preview,
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
            f.run("unreachable", &preview, &|_| panic!())
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
        let e = f.run("unreachable", &preview, &|_| panic!()).unwrap_err();
        assert!(e.message.contains("review the new preview"));
        assert!(e.written.is_empty());
        assert!(!preview[0].path.exists());
        assert!(f.preview()[1].exists);
    }

    const BODY: &str = "const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'body', bars: 1, loop: true }] };\nsong(meta, { body: note('a4').s('sine') })";
    const INTRO_BODY: &str = "const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'intro', bars: 1 }, { name: 'body', bars: 1, loop: true }] };\nsong(meta, { intro: note('a3').s('sine'), body: note('a4').s('sine') })";

    #[test]
    fn split_wavs_include_the_first_pass_and_encoded_loop_markers() {
        let f = Fixture::new();
        for (source, opening_frames) in [(INTRO_BODY, 22_050), (BODY, 11_025)] {
            let preview = f.preview();
            let events = RefCell::new(Vec::new());
            let result = f
                .run(source, &preview, &|event| events.borrow_mut().push(event))
                .unwrap();
            let [opening, repeat] = &result.files[..] else {
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
                .run(BODY, &preview, &|p| {
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
