//! Pre-master stereo WAV sets. Every pair comes from the same render and uses the same join.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use strudeland_engine::{StemId, wav};
use strudeland_host::{ChunkWindow, PlannedWindow};
use strudeland_project::{Project, RenderSet, render_filename};

use crate::output::{io_error, render_suffix};
use crate::{
    AudioMode, Format, LengthRequest, PipelineAudio, Progress, RenderError, RenderOptions, Summary,
    ms_since, render_audio_for,
};

const MAX_STEMS: usize = 128;
const AUDIO_BUDGET_BYTES: u64 = 512 << 20;
const MANIFEST_NAME: &str = "stems.json";
const MIX_NAME: &str = "mix.wav";
const STEREO_WAV_HEADER_BYTES: usize = 44;

/// How the musical outputs of a stem export are grouped.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StemGrouping {
    Orbit,
    Voice,
    Instrument,
}

impl From<StemGrouping> for strudeland_engine::StemGrouping {
    fn from(group: StemGrouping) -> Self {
        match group {
            StemGrouping::Orbit => Self::Orbit,
            StemGrouping::Voice => Self::Voice,
            StemGrouping::Instrument => Self::Instrument,
        }
    }
}

#[derive(Debug)]
pub struct StemExport {
    pub directory: PathBuf,
    /// Absolute paths in publication order, including the mix and manifest.
    pub files: Vec<PathBuf>,
    pub summary: Summary,
}

/// Destination of a set. Previewing the path does not reserve it.
#[derive(Debug)]
pub struct StemTarget {
    path: PathBuf,
    destination: Destination,
}

#[derive(Debug)]
enum Destination {
    Explicit,
    ChosenFolder,
    Project(Project),
}

impl StemTarget {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Check whether the destination could be reserved, without changing the filesystem.
    pub fn check_available(&self) -> Result<(), RenderError> {
        if let Destination::Project(project) = &self.destination {
            let Some(name) = self.path.file_name().and_then(|s| s.to_str()) else {
                return Err(bad_output("a stem set needs a directory name"));
            };
            project
                .render_set_path(name)
                .map_err(|e| io_error(&self.path, e))?;
        } else {
            self.check_parent()?;
        }
        match fs::symlink_metadata(&self.path) {
            Ok(_) => Err(io_at(
                &self.path,
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "the stem set destination already exists",
                ),
            )),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_at(&self.path, e)),
        }
    }

    fn check_parent(&self) -> Result<(), RenderError> {
        let parent = self
            .path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        check_directory(
            parent,
            matches!(self.destination, Destination::ChosenFolder),
        )
    }

    fn reserve(&self) -> Result<RenderSet, RenderError> {
        let set = match &self.destination {
            Destination::Project(project) => project.create_render_set(&self.path),
            Destination::Explicit | Destination::ChosenFolder => {
                self.check_parent()?;
                RenderSet::reserve(&self.path)
            }
        }
        .map_err(|e| io_error(&self.path, e))?;
        if matches!(self.destination, Destination::ChosenFolder) {
            check_directory(&self.path, true)?;
        }
        Ok(set)
    }
}

/// Resolve an explicit set directory or the default directory of a song within a project.
pub fn resolve_stem_output(
    out: Option<&Path>,
    song: &Path,
    length: &LengthRequest,
) -> Result<StemTarget, RenderError> {
    if let Some(path) = out {
        if path.file_name().is_none() {
            return Err(bad_output("a stem set needs a directory name for --out"));
        }
        return Ok(StemTarget {
            path: path.to_owned(),
            destination: Destination::Explicit,
        });
    }
    let (project, stem) = Project::find(song).ok_or_else(|| {
        bad_output(format!(
            "--out is required: {} is not in a strudeland project",
            song.display()
        ))
    })?;
    project_target(&project, &stem, length)
}

/// Resolve the default path under an already-open project, including for an unsaved editor buffer.
pub fn resolve_project_stem_output(
    project: &Project,
    song: &Path,
    length: &LengthRequest,
) -> Result<StemTarget, RenderError> {
    project_target(project, song_stem(song)?, length)
}

/// Resolve the default set name beneath an explicitly chosen parent folder.
pub fn resolve_stem_output_in(
    parent: &Path,
    song: &Path,
    length: &LengthRequest,
) -> Result<StemTarget, RenderError> {
    let parent = parent.canonicalize().map_err(|e| io_at(parent, e))?;
    let path = parent.join(set_name(song_stem(song)?, length)?);
    Ok(StemTarget {
        path,
        destination: Destination::ChosenFolder,
    })
}

fn song_stem(song: &Path) -> Result<&str, RenderError> {
    song.file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| bad_output("a stem set needs a valid song name"))
}

fn set_name(stem: &str, length: &LengthRequest) -> Result<String, RenderError> {
    let suffix =
        render_suffix(length).map_or_else(|| "stems".into(), |range| format!("{range}-stems"));
    let name =
        render_filename(stem, Some(&suffix), "wav").map_err(|e| bad_output(e.to_string()))?;
    Ok(Path::new(&name)
        .with_extension("")
        .to_string_lossy()
        .into_owned())
}

pub(crate) fn project_target(
    project: &Project,
    stem: &str,
    length: &LengthRequest,
) -> Result<StemTarget, RenderError> {
    let name = set_name(stem, length)?;
    let path = project
        .render_set_path(&name)
        .map_err(|e| io_error(&project.root().join("renders").join(&name), e))?;
    Ok(StemTarget {
        path,
        destination: Destination::Project(project.clone()),
    })
}

fn bad_output(message: impl Into<String>) -> RenderError {
    RenderError::Usage {
        flag: "--out",
        message: message.into(),
    }
}

fn io_at(path: &Path, source: io::Error) -> RenderError {
    RenderError::Io {
        path: path.to_owned(),
        source,
    }
}

fn check_directory(path: &Path, canonical: bool) -> Result<(), RenderError> {
    if !fs::metadata(path).map_err(|e| io_at(path, e))?.is_dir() {
        return Err(io_at(
            path,
            io::Error::new(io::ErrorKind::NotADirectory, "not a directory"),
        ));
    }
    if canonical && path.canonicalize().map_err(|e| io_at(path, e))? != path {
        return Err(io_at(
            path,
            io::Error::other("the chosen output directory changed"),
        ));
    }
    Ok(())
}

fn stereo_data_size(frames: u64) -> Result<u32, RenderError> {
    usize::try_from(frames)
        .ok()
        .and_then(|frames| wav::data_size(2, frames))
        .ok_or_else(|| RenderError::Encode("a stem WAV would exceed the RIFF size limit".into()))
}

/// Conservative peak for joined PCM, channel buffers in flight and the single WAV being encoded.
pub(crate) fn validate_budget(
    stem_count: usize,
    frames: u64,
    chunks: &[ChunkWindow],
    workers: usize,
) -> Result<usize, RenderError> {
    if stem_count > MAX_STEMS {
        return Err(RenderError::Render(format!(
            "the set has {stem_count} stems, above the limit of {MAX_STEMS}"
        )));
    }
    let channels = 2 * (1 + stem_count);
    let err = || {
        RenderError::Render(
        "stem audio exceeds the 512 MiB buffer limit; select a shorter range or use fewer workers".into(),
    )
    };
    let wav_size = stereo_data_size(frames)?;
    let chunk_frames = chunks.iter().map(|chunk| chunk.frames).max().unwrap_or(0);
    let in_flight = chunk_frames
        .checked_mul(workers as u64 + 1)
        .ok_or_else(err)?;
    let pcm = frames
        .checked_add(in_flight)
        .and_then(|f| f.checked_mul(channels as u64 * 4))
        .ok_or_else(err)?;
    // Include per-worker planar sums and the fixed stereo accumulator for shared FX.
    let scratch =
        workers as u64 * (channels as u64 * strudeland_engine::QUANTUM as u64 * 4 + (32 << 10));
    let extra = scratch + u64::from(wav_size) + (1 << 16);
    if pcm
        .checked_add(extra)
        .is_none_or(|bytes| bytes > AUDIO_BUDGET_BYTES)
    {
        return Err(err());
    }
    Ok(channels)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    version: u8,
    grouping: StemGrouping,
    mix: &'static str,
    window: PlannedWindow,
    sample_rate: u32,
    frames: u64,
    stems: Vec<ManifestStem>,
}

#[derive(Serialize)]
struct ManifestStem {
    file: String,
    #[serde(flatten)]
    group: ManifestGroup,
}

#[derive(Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
enum ManifestGroup {
    Orbit(String),
    Voice(String),
    Unlabeled,
    Instrument(String),
    Effects(String),
}

impl From<&StemId> for ManifestGroup {
    fn from(id: &StemId) -> Self {
        match id {
            StemId::Orbit(id) => Self::Orbit(id.clone()),
            StemId::Voice(id) => Self::Voice(id.clone()),
            StemId::Unlabeled => Self::Unlabeled,
            StemId::Instrument(id) => Self::Instrument(id.clone()),
            StemId::Effects(id) => Self::Effects(id.clone()),
        }
    }
}

/// Renders a standard window as a pre-master float WAV set. The mastering and encoder options
/// in `opts` apply to mix exports. A stem set always writes raw 32-bit float PCM.
pub fn render_stems(
    code: &str,
    opts: &RenderOptions,
    grouping: StemGrouping,
    target: &StemTarget,
    progress: &dyn Fn(Progress),
) -> Result<StemExport, RenderError> {
    let started = Instant::now();
    target.check_available()?;
    let PipelineAudio { audio, stems, .. } =
        render_audio_for(code, opts, AudioMode::Stems(grouping), progress)?;
    let layout = stems.ok_or_else(|| RenderError::Render("missing stem layout".into()))?;
    let mut summary = audio.summary;
    let manifest = Manifest {
        version: 1,
        grouping,
        mix: MIX_NAME,
        window: summary.window,
        sample_rate: summary.sample_rate,
        frames: summary.frames,
        stems: layout
            .ids()
            .iter()
            .enumerate()
            .map(|(index, id)| ManifestStem {
                file: format!("stem-{:03}.wav", index + 1),
                group: id.into(),
            })
            .collect(),
    };
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|e| RenderError::Encode(format!("stem manifest: {e}")))?;
    manifest_bytes.push(b'\n');
    let capacity = stereo_data_size(summary.frames)?;
    let mut bytes = Vec::with_capacity(capacity as usize + STEREO_WAV_HEADER_BYTES);

    let set = target.reserve()?;
    let mut files = Vec::with_capacity(manifest.stems.len() + 2);
    let mut written = |name, bytes: &[u8]| -> Result<(), RenderError> {
        let path = target.path().join(name);
        set.create_file(name, bytes)
            .map_err(|e| io_error(&path, e))?;
        let path = std::path::absolute(&path).unwrap_or(path);
        progress(Progress::Written { path: path.clone() });
        files.push(path);
        Ok(())
    };
    let mut bytes_written = manifest_bytes.len() as u64;
    let mut encode_ms = 0.0;
    let names = std::iter::once(MIX_NAME).chain(manifest.stems.iter().map(|s| s.file.as_str()));
    for (name, channels) in names.zip(audio.channels.chunks_exact(2)) {
        let encoding = Instant::now();
        bytes.clear();
        wav::write_f32(&mut bytes, channels, summary.sample_rate)
            .map_err(|e| RenderError::Encode(format!("{name}: {e}")))?;
        encode_ms += ms_since(encoding);
        bytes_written += bytes.len() as u64;
        written(name, &bytes)?;
    }
    progress(Progress::Encoded { pass: 1 });
    written(MANIFEST_NAME, &manifest_bytes)?;
    set.check_current()
        .map_err(|e| io_error(target.path(), e))?;
    summary.format = Some(Format::Wav);
    summary.bytes = Some(bytes_written);
    summary.timings.encode_ms = encode_ms;
    summary.timings.total_ms = ms_since(started);
    let directory = std::path::absolute(target.path()).unwrap_or_else(|_| target.path().to_owned());
    Ok(StemExport {
        directory,
        files,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_refuses_too_many_groups_large_audio_and_oversized_riff() {
        assert_eq!(validate_budget(7, 44_100, &[], 2).unwrap(), 16);
        assert!(matches!(
            validate_budget(129, 0, &[], 1),
            Err(RenderError::Render(_))
        ));
        assert!(matches!(
            validate_budget(128, 1 << 20, &[], 4),
            Err(RenderError::Render(_))
        ));
        assert!(matches!(
            validate_budget(1, 1 << 29, &[], 1),
            Err(RenderError::Encode(_))
        ));
    }
}
