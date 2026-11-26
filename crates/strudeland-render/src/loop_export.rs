//! Export presets: trims a one-shot or splices a loop, then finishes the files.

use std::time::Instant;

use serde::Serialize;

use crate::encode::{Mp3Flush, Real};
use crate::finish::{Finishing, Leveling, finish_files};
use crate::splice::{FileCut, splice};
use crate::{
    AudioMode, ExportPreset, OutputOptions, PipelineAudio, Progress, RenderError, RenderOptions,
    RenderOutput, RenderedAudio, ms_since, render_audio_for,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportRole {
    Intro,
    Loop,
    Single,
}

/// The forward loop in decoded PCM. The end is exclusive when computed as start plus length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopPoints {
    pub start_frame: u64,
    pub length_frames: u64,
    pub sample_rate: u32,
}

impl LoopPoints {
    pub fn godot_loop_offset(&self) -> f64 {
        self.start_frame as f64 / f64::from(self.sample_rate)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportFile {
    pub role: ExportRole,
    pub output: RenderOutput,
    pub loop_points: Option<LoopPoints>,
}

/// Plans, renders, and encodes every file for the given preset before returning any of them.
pub fn render_export(
    code: &str,
    opts: &RenderOptions,
    preset: ExportPreset,
    progress: &dyn Fn(Progress),
) -> Result<Vec<ExportFile>, RenderError> {
    finish_export(
        render_audio_for(code, opts, AudioMode::Preset(preset), progress)?,
        preset,
        &opts.output,
        progress,
    )
}

/// Finishes rendered export audio: trims a one-shot's silent post-roll, or splices the loop and
/// levels every file with one static gain.
pub(crate) fn finish_export(
    pipeline: PipelineAudio,
    preset: ExportPreset,
    out: &OutputOptions,
    progress: &dyn Fn(Progress),
) -> Result<Vec<ExportFile>, RenderError> {
    let PipelineAudio {
        mut audio,
        loop_audio,
        ..
    } = pipeline;
    if preset == ExportPreset::OneShot {
        trim_silent_post_roll(&mut audio)?;
        return finish_files(
            Finishing {
                cuts: vec![FileCut::whole(&audio)],
                summary: audio.summary.clone(),
                leveling: Leveling::Limited,
                mp3_flush: Mp3Flush::Gap,
            },
            out,
            &Real,
            progress,
        );
    }

    let loop_audio = loop_audio.ok_or_else(|| render_error("the plan has no loop start"))?;
    let measure_start = Instant::now();
    let spliced = splice(
        audio.channels,
        audio.music_frames,
        audio.summary.window,
        audio.summary.sample_rate,
        loop_audio,
        preset,
    )?;
    let mut summary = audio.summary;
    summary.timings.level_ms = ms_since(measure_start);
    finish_files(
        Finishing {
            cuts: spliced.cuts,
            summary,
            leveling: Leveling::RepeatSafeStatic {
                program: spliced.program,
                peak_db: spliced.peak_db,
            },
            mp3_flush: Mp3Flush::Gap,
        },
        out,
        &Real,
        progress,
    )
}

fn render_error(message: &str) -> RenderError {
    RenderError::Render(message.into())
}

fn trim_silent_post_roll(audio: &mut RenderedAudio) -> Result<(), RenderError> {
    let started = Instant::now();
    let musical = usize::try_from(audio.music_frames)
        .map_err(|_| render_error("one-shot window is too large"))?;
    // Seek back from the end so quiet gaps before late echoes stay in the output.
    let tail = audio
        .channels
        .iter()
        .filter_map(|channel| {
            channel
                .get(musical..)?
                .iter()
                .rposition(|&sample| sample != 0.0)
        })
        .max();
    let frames = tail.map_or(musical, |last| musical + last + 1);
    for channel in &mut audio.channels {
        channel.truncate(frames);
    }
    audio.summary.frames = frames as u64;
    audio.summary.seconds = frames as f64 / f64::from(audio.summary.sample_rate);
    let trim_ms = ms_since(started);
    audio.summary.timings.render_ms += trim_ms;
    audio.summary.timings.total_ms += trim_ms;
    Ok(())
}
