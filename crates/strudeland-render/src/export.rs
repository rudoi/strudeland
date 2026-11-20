//! Arranges rendered PCM into export files and finishes repeatable regions with one static gain.

use std::ops::Range;
use std::time::Instant;

use serde::Serialize;

use crate::encode::{CORRECTION_MARGIN_DB, MAX_ENCODES, encode_once_with_loop, opus, verify};
use crate::join::window_frames;
use crate::level::true_peak::TAPS_PER_PHASE;
use crate::level::{LoudnessSummary, Measure, check_input, measure, true_peak_db};
use crate::{
    ExportPreset, Format, LoopAudio, Loudness, OutputOptions, Progress, RenderError, RenderOptions,
    RenderOutput, RenderedAudio, Summary, finish_file, ms_since, render_audio_for,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
    let (mut audio, loop_audio) = render_audio_for(code, opts, Some(preset), progress)?;
    if preset == ExportPreset::OneShot {
        trim_silent_post_roll(&mut audio)?;
        return Ok(vec![ExportFile {
            role: ExportRole::Single,
            output: finish_file(&audio, &opts.output, progress, true)?,
            loop_points: None,
        }]);
    }

    let window = audio.summary.window;
    let LoopAudio {
        start_bar,
        isolated_body,
    } = loop_audio.ok_or_else(|| render_error("the plan has no loop start"))?;
    if !(window.from_bar..window.to_bar).contains(&start_bar) {
        return Err(render_error("loop start is outside the planned window"));
    }
    let start = usize::try_from(window_frames(window.from_bar, start_bar, audio.cps))
        .map_err(|_| render_error("loop start is too large"))?;
    let end = usize::try_from(window_frames(window.from_bar, window.to_bar, audio.cps))
        .map_err(|_| render_error("loop end is too large"))?;
    if end <= start {
        return Err(render_error("loop has no frames"));
    }
    let measure_start = Instant::now();
    let sample_rate = audio.summary.sample_rate;
    let body_frames = end - start;
    let combined_frames = end
        .checked_add(body_frames)
        .ok_or_else(|| render_error("the combined loop is too large"))?;
    let repeat_range = end..combined_frames;
    let (mut opening, mut repeat) = match isolated_body {
        Some(body) if start > 0 => (audio.channels, body),
        None if start == 0 => {
            let opening = audio
                .channels
                .iter()
                .map(|ch| copy_opening(ch, end, combined_frames))
                .collect::<Result<Vec<_>, _>>()?;
            (opening, audio.channels)
        }
        _ => return Err(render_error("the plan has inconsistent loop history")),
    };
    fold_tail(&mut repeat, body_frames)?;
    join_opening(&mut opening, &repeat, end, sample_rate)?;

    check_input(&opening)?;
    let program = measure(&opening, sample_rate);
    let wrap = wrap_peak(&repeat);
    let loop_peak = program.true_peak_db.max(wrap);
    let parts = match preset {
        ExportPreset::IntroLoop => {
            for ch in &mut opening {
                ch.truncate(end);
            }
            vec![
                Part {
                    role: ExportRole::Intro,
                    channels: opening,
                    bars: window.from_bar..window.to_bar,
                    loop_range: None,
                },
                Part {
                    role: ExportRole::Loop,
                    channels: repeat,
                    bars: start_bar..window.to_bar,
                    loop_range: Some(0..body_frames),
                },
            ]
        }
        ExportPreset::FoldedLoop | ExportPreset::TaggedLoop => {
            drop(repeat);
            vec![Part {
                role: if preset == ExportPreset::FoldedLoop {
                    ExportRole::Loop
                } else {
                    ExportRole::Single
                },
                channels: opening,
                bars: window.from_bar..window.to_bar,
                loop_range: Some(repeat_range),
            }]
        }
        ExportPreset::OneShot => return Err(render_error("unexpected one-shot preset")),
    };
    let mut summary = audio.summary;
    summary.timings.level_ms = ms_since(measure_start);
    finish_loop(parts, summary, &opts.output, program, loop_peak, progress)
}

fn render_error(message: &str) -> RenderError {
    RenderError::Render(message.into())
}

fn copy_opening(
    channel: &[f32],
    frames: usize,
    total_frames: usize,
) -> Result<Vec<f32>, RenderError> {
    let opening = channel
        .get(..frames)
        .ok_or_else(|| render_error("opening extends past the rendered PCM"))?;
    let mut copy = Vec::new();
    copy.try_reserve_exact(total_frames)
        .map_err(|_| render_error("cannot allocate the loop opening"))?;
    copy.extend_from_slice(opening);
    Ok(copy)
}

/// Bring the last few milliseconds of the opening to the same phase and effect
/// state as the repeat's end. The repeat stays independent of the opening.
fn join_opening(
    opening: &mut [Vec<f32>],
    repeat: &[Vec<f32>],
    opening_frames: usize,
    sample_rate: u32,
) -> Result<(), RenderError> {
    if opening.len() != repeat.len() {
        return Err(render_error("opening and repeat channel counts differ"));
    }
    for (first, periodic) in opening.iter_mut().zip(repeat) {
        if first.len() != opening_frames || first.len() < periodic.len() {
            return Err(render_error("invalid loop opening length"));
        }
        // 10 ms, limited to the latter half of even a very short loop.
        let blend_frames = (sample_rate as usize / 100).min(periodic.len() / 2);
        let fade = first.iter_mut().rev().zip(periodic.iter().rev());
        for (i, (dst, &src)) in fade.take(blend_frames).enumerate() {
            let x = 1.0 - i as f64 / blend_frames as f64;
            let weight = x * x * (3.0 - 2.0 * x);
            *dst = if i == 0 {
                src
            } else {
                let a = f64::from(*dst);
                (a + (f64::from(src) - a) * weight) as f32
            };
        }
        first
            .try_reserve_exact(periodic.len())
            .map_err(|_| render_error("cannot allocate the combined loop export"))?;
        first.extend_from_slice(periodic);
    }
    Ok(())
}

fn trim_silent_post_roll(audio: &mut RenderedAudio) -> Result<(), RenderError> {
    let started = Instant::now();
    let window = audio.summary.window;
    let musical = usize::try_from(window_frames(window.from_bar, window.to_bar, audio.cps))
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

fn fold_tail(channels: &mut [Vec<f32>], body_frames: usize) -> Result<(), RenderError> {
    if body_frames == 0 {
        return Err(render_error("cannot fold into an empty loop"));
    }
    for channel in channels {
        let (body, tail) = channel
            .split_at_mut_checked(body_frames)
            .ok_or_else(|| render_error("loop extends past the rendered PCM"))?;
        for cycle in tail.chunks(body_frames) {
            for (sample, tail) in body.iter_mut().zip(cycle) {
                *sample += *tail;
            }
        }
        channel.truncate(body_frames);
    }
    Ok(())
}

/// Measure cyclic FIR input across the folded body's join. Each channel has at
/// least one frame. Cycling the edges covers loops shorter than a tap window.
fn wrap_peak(channels: &[Vec<f32>]) -> f64 {
    let mut edges = Vec::with_capacity(channels.len());
    for body in channels {
        let width = TAPS_PER_PHASE;
        let preceding = (body.len() - width % body.len()) % body.len();
        edges.push(
            body.iter()
                .skip(preceding)
                .chain(body.iter().cycle())
                .take(2 * width)
                .copied()
                .collect(),
        );
    }
    true_peak_db(&edges)
}

struct Part {
    role: ExportRole,
    channels: Vec<Vec<f32>>,
    bars: Range<u32>,
    loop_range: Option<Range<usize>>,
}

fn finish_loop(
    parts: Vec<Part>,
    summary: Summary,
    options: &OutputOptions,
    program: Measure,
    program_peak: f64,
    progress: &dyn Fn(Progress),
) -> Result<Vec<ExportFile>, RenderError> {
    let sample_rate = summary.sample_rate;
    let format = options.encoder.format();
    let started = Instant::now();
    let before = if parts.len() == 1 {
        vec![program]
    } else {
        parts
            .iter()
            .map(|part| measure(&part.channels, sample_rate))
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
        let mut files = Vec::with_capacity(parts.len());
        let mut worst_peak = f64::NEG_INFINITY;
        for (part, before) in parts.iter().zip(&before) {
            let started = Instant::now();
            let gained;
            let channels = if gain_db == 0.0 {
                &part.channels
            } else {
                let gain = 10f64.powf(gain_db / 20.0);
                gained = part
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
            let (range, loop_points) = match &part.loop_range {
                Some(range) => {
                    let (range, rate) = if format == Format::Opus {
                        (
                            opus::resampled_len(range.start, sample_rate)
                                ..opus::resampled_len(range.end, sample_rate),
                            opus::OPUS_RATE,
                        )
                    } else {
                        (range.clone(), sample_rate)
                    };
                    let points = LoopPoints {
                        start_frame: range.start as u64,
                        length_frames: range.len() as u64,
                        sample_rate: rate,
                    };
                    (Some(range.start as u64..range.end as u64), Some(points))
                }
                None => (None, None),
            };
            let bytes = encode_once_with_loop(channels, sample_rate, &options.encoder, range)?;
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
            summary.window.from_bar = part.bars.start;
            summary.window.to_bar = part.bars.end;
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
                role: part.role,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_full_and_partial_repeats_into_a_body() {
        let mut channels = vec![vec![1., 2., 3., 4., 5., 6., 7.], vec![0.; 7]];
        fold_tail(&mut channels, 2).unwrap();
        assert_eq!(channels, [vec![16., 12.], vec![0.; 2]]);
        assert!(fold_tail(&mut channels, 0).is_err());
        assert!(fold_tail(&mut channels, 3).is_err());
    }

    #[test]
    fn the_opening_blend_matches_the_repeat_without_a_step_in_the_fade() {
        let mut opening = vec![vec![0.0; 30]];
        let repeat = vec![vec![1.0; 24]];
        join_opening(&mut opening, &repeat, 30, 1000).unwrap();
        let first = &opening[0];
        assert!(first[..20].iter().all(|&sample| sample == 0.0));
        assert!(first[19..30].windows(2).all(|pair| {
            let step = pair[1] - pair[0];
            step > 0.0 && step < 0.15
        }));
        assert_eq!(first[29], 1.0);
        assert_eq!(&first[30..], &repeat[0]);
    }

    #[test]
    fn measures_a_wrap_even_for_a_loop_shorter_than_the_filter() {
        let body = vec![vec![-0.9, 0.0]];
        let peak = wrap_peak(&body);
        assert!(peak.is_finite());
        assert!(peak > true_peak_db(&body));
    }
}
