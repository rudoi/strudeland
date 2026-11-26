//! Arranges rendered PCM into export files and finishes repeatable regions with one static gain.

use std::time::Instant;

use serde::Serialize;

use crate::encode::{
    CORRECTION_MARGIN_DB, EncodeRequest, MAX_ENCODES, Mp3Flush, encode_once, loop_points, verify,
};
use crate::level::{LoudnessSummary, Measure, measure};
use crate::splice::{FileCut, splice};
use crate::{
    AudioMode, ExportPreset, Format, Loudness, OutputOptions, PipelineAudio, Progress, RenderError,
    RenderOptions, RenderOutput, RenderedAudio, Summary, finish_file, ms_since, render_audio_for,
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
        return Ok(vec![ExportFile {
            role: ExportRole::Single,
            output: finish_file(&audio, out, progress, true)?,
            loop_points: None,
        }]);
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
    finish_loop(
        spliced.cuts,
        summary,
        out,
        spliced.program,
        spliced.peak_db,
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

fn finish_loop(
    cuts: Vec<FileCut<'_>>,
    summary: Summary,
    options: &OutputOptions,
    program: Measure,
    program_peak: f64,
    progress: &dyn Fn(Progress),
) -> Result<Vec<ExportFile>, RenderError> {
    let sample_rate = summary.sample_rate;
    let format = options.encoder.format();
    let started = Instant::now();
    let before = if cuts.len() == 1 {
        vec![program]
    } else {
        cuts.iter()
            .map(|cut| measure(&cut.channels, sample_rate))
            .collect()
    };
    let mut level_ms = summary.timings.level_ms + ms_since(started);
    // Playing a split asset by itself pads its edges with silence during peak measurement.
    // Account for those edges as well as the continuous intro handoff and the loop wrap.
    let program_peak = before
        .iter()
        .fold(program_peak, |peak, file| peak.max(file.true_peak_db));
    let mut warnings = Vec::new();
    let (target, ceiling, mut gain_db) = match options.loudness {
        Loudness::Off => {
            if program_peak > 0.0 {
                warnings.push(format!("loudness off: true peak is {program_peak:.1} dBTP (above 0 dBFS); the file may clip"));
            }
            (None, None, 0.0)
        }
        Loudness::On {
            target_lufs,
            true_peak_db,
        } => {
            let ideal = if program.lufs.is_finite() {
                target_lufs - program.lufs
            } else {
                warnings.push(
                    "loudness: skipped normalization because integrated LUFS is unmeasurable; the true-peak ceiling still applies"
                        .to_owned(),
                );
                0.0
            };
            (
                Some(target_lufs),
                Some(true_peak_db),
                ideal.min(true_peak_db - program_peak),
            )
        }
    };
    if format == Format::Mp3 {
        warnings.push("mp3 encoder delay and padding can cause gaps in loop playback; use WAV for sample-exact loops".into());
    }
    let mut encode_ms = 0.0;
    for pass in 1..=MAX_ENCODES {
        let mut files = Vec::with_capacity(cuts.len());
        let mut worst_peak = f64::NEG_INFINITY;
        for (cut, before) in cuts.iter().zip(&before) {
            let started = Instant::now();
            let gained;
            let channels = if gain_db == 0.0 {
                &cut.channels
            } else {
                let gain = 10f64.powf(gain_db / 20.0);
                gained = cut
                    .channels
                    .iter()
                    .map(|ch| ch.iter().map(|&x| (f64::from(x) * gain) as f32).collect())
                    .collect();
                &gained
            };
            let after = if gain_db == 0.0 {
                *before
            } else {
                measure(channels, sample_rate)
            };
            level_ms += ms_since(started);

            let started = Instant::now();
            let (range, loop_points) = cut
                .loop_range
                .as_ref()
                .map(|r| loop_points(format, r, sample_rate))
                .unzip();
            let bytes = encode_once(&EncodeRequest {
                channels,
                sample_rate,
                encoder: &options.encoder,
                mp3_flush: Mp3Flush::Gap,
                loop_range: range,
            })?;
            let decoded_peak = if format == Format::Wav {
                None
            } else {
                let peak = verify::decoded_true_peak_db(format, &bytes)?;
                worst_peak = worst_peak.max(peak);
                Some(peak)
            };
            encode_ms += ms_since(started);

            let frames = channels.first().map_or(0, Vec::len) as u64;
            let mut summary = summary.clone();
            summary.window.from_bar = cut.bars.start;
            summary.window.to_bar = cut.bars.end;
            summary.frames = frames;
            summary.seconds = frames as f64 / f64::from(sample_rate);
            summary.format = Some(format);
            summary.bytes = Some(bytes.len() as u64);
            summary.loudness = Some(LoudnessSummary {
                enabled: target.is_some(),
                target_lufs: target,
                ceiling_dbtp: ceiling,
                limiter_ceiling_dbtp: None,
                input_lufs: before.lufs,
                input_true_peak_db: before.true_peak_db,
                gain_db,
                limiter_samples: 0,
                output_lufs: after.lufs,
                output_true_peak_db: after.true_peak_db,
                decoded_true_peak_db: decoded_peak,
                encodes: pass,
            });
            files.push(ExportFile {
                role: cut.role,
                output: RenderOutput { bytes, summary },
                loop_points,
            });
        }
        progress(Progress::Encoded { pass });
        if let Some(ceiling) = ceiling
            && worst_peak > ceiling
        {
            if pass < MAX_ENCODES {
                gain_db -= worst_peak - ceiling + CORRECTION_MARGIN_DB;
                continue;
            }
            warnings.push(format!("lossy encode: decoded true peak {worst_peak:.2} dBTP is above the ceiling {ceiling:.1} dBTP after {} corrections", MAX_ENCODES - 1));
        }
        for file in &mut files {
            let summary = &mut file.output.summary;
            summary.timings.level_ms = level_ms;
            summary.timings.encode_ms = encode_ms;
            summary.timings.total_ms += level_ms + encode_ms;
            summary.warnings.extend(warnings.iter().cloned());
        }
        return Ok(files);
    }
    Err(render_error("no export encode pass was attempted"))
}
