//! strudeland's render pipeline: a song file to finished audio without the app or a browser.
//!
//! 1. **Plan**: the headless host evaluates the song once and plans every chunk's engine job in
//!    one call ([`strudeland_host::Host::plan_render`]).
//! 2. **Check**: a song that uses sounds or controls the engine cannot play is refused before
//!    anything is rendered ([`RenderError::Unsupported`]).
//! 3. **Render**: the chunk jobs render in parallel on a scoped worker pool, one engine per
//!    worker ([`strudeland_engine::render`]).
//! 4. **Join**: each chunk's lookback head and overlap tail are dropped and the contents are
//!    copied sample-exactly into the window as the chunks finish ([`join`]).
//! 5. **Finish**: level, encode, verify and correct each file cut ([`finish`], [`render_export`]).
//!
//! Nothing here may panic on data from the song, the plan or a file: the release profile is
//! `panic = "abort"`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod bench;
#[cfg(test)]
mod characterization;
pub mod encode;
mod error;
pub mod export;
mod finish;
pub mod join;
pub mod level;
mod loop_export;
mod options;
pub mod output;
mod pipeline_audio;
mod plan;
mod render;
mod splice;
mod stems;
mod summary;
#[doc(hidden)]
pub mod testing;

use std::time::Instant;

use strudeland_engine::StemLayout;

use pipeline_audio::{EngineChunks, PlanTiming, render_planned};

pub use error::{ExitClass, RenderError, Unsupported};
pub use loop_export::{ExportFile, ExportRole, LoopPoints, render_export};
pub use options::{
    BitDepth, DEFAULT_MAX_JOBS, DEFAULT_TARGET_LUFS, DEFAULT_TRUE_PEAK_DB, EncoderOptions, Format,
    Loudness, Mp3Mode, OnOff, OutputOptions, RenderOptions,
};
pub use output::{OutputTarget, resolve_format, write_output};
pub use render::MAX_WARNINGS;
pub use stems::{StemExport, StemGrouping, StemTarget};
pub use strudeland_host::{
    ExportPreset, HostError, LengthProblem, LengthRequest, MAX_BARS, MAX_TIMEOUT_SEC,
    timeout_from_secs,
};
pub use summary::{Progress, RenderOutput, RenderedAudio, Summary, Timings};

/// Plans, renders, joins, levels and encodes. Equivalent to `finish(&render_audio(..)?, ..)`.
pub fn render(
    code: &str,
    opts: &RenderOptions,
    progress: &dyn Fn(Progress),
) -> Result<RenderOutput, RenderError> {
    finish(&render_audio(code, opts, progress)?, &opts.output, progress)
}

/// Steps 1 to 4 of the pipeline: plan, refuse, render the chunks, join. No leveling, no encoding.
pub fn render_audio(
    code: &str,
    opts: &RenderOptions,
    progress: &dyn Fn(Progress),
) -> Result<RenderedAudio, RenderError> {
    render_audio_for(code, opts, AudioMode::Standard, progress).map(|result| result.audio)
}

struct LoopAudio {
    start_bar: u32,
    start_frame: u64,
    isolated_body: Option<Vec<Vec<f32>>>,
}

enum AudioMode {
    Standard,
    Preset(ExportPreset),
    Stems(StemGrouping),
}

struct PipelineAudio {
    audio: RenderedAudio,
    loop_audio: Option<LoopAudio>,
    stems: Option<StemLayout>,
}

fn render_audio_for(
    code: &str,
    opts: &RenderOptions,
    mode: AudioMode,
    progress: &dyn Fn(Progress),
) -> Result<PipelineAudio, RenderError> {
    let started = Instant::now();
    render::validate_jobs(opts.jobs)?;
    let preset = match mode {
        AudioMode::Preset(preset) => Some(preset),
        _ => None,
    };
    let (planned, plan_ms) = plan::plan(code, opts, preset)?;
    render_planned(
        planned,
        PlanTiming { started, plan_ms },
        opts,
        mode,
        &EngineChunks,
        progress,
    )
}

/// Step 5 (finish): level and encode a joined buffer, verifying and correcting a lossy file's
/// decoded true peak, through the finishing loop with limited leveling (the static gain, then the
/// true-peak limiter). Callable several times on one [`RenderedAudio`]. The summary starts from
/// `audio.summary` and gains the format, size, loudness and timings; level and encoder warnings
/// follow the render's.
pub fn finish(
    audio: &RenderedAudio,
    out: &OutputOptions,
    progress: &dyn Fn(Progress),
) -> Result<RenderOutput, RenderError> {
    let files = finish::finish_files(
        finish::Finishing {
            cuts: vec![splice::FileCut::whole(audio)],
            summary: audio.summary.clone(),
            leveling: finish::Leveling::Limited,
            mp3_flush: encode::Mp3Flush::NoGap,
        },
        out,
        &encode::Real,
        progress,
    )?;
    let mut files = files.into_iter();
    match (files.next(), files.next()) {
        (Some(file), None) => Ok(file.output),
        _ => Err(RenderError::render(
            "finishing did not return exactly one file",
        )),
    }
}

fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
