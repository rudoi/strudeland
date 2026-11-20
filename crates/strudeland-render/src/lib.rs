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
//! 5. **Level** and 6. **encode** ([`finish`]).
//!
//! Nothing here may panic on data from the song, the plan or a file: the release profile is
//! `panic = "abort"`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)
)]

pub mod bench;
pub mod encode;
mod error;
mod export;
pub mod join;
pub mod level;
mod options;
pub mod output;
mod plan;
mod render;
mod summary;

use std::time::Instant;

use strudeland_engine::{EngineJob, Rendered};
use strudeland_host::{ChunkWindow, PlannedRender, PlannedWindow, SAMPLE_RATE};

pub use error::{ExitClass, RenderError, Unsupported};
pub use export::{ExportFile, ExportRole, LoopPoints, render_export};
pub use options::{
    BitDepth, DEFAULT_MAX_JOBS, DEFAULT_TARGET_LUFS, DEFAULT_TRUE_PEAK_DB, EncoderOptions, Format,
    Loudness, Mp3Mode, OnOff, OutputOptions, RenderOptions,
};
pub use output::{
    OutputTarget, resolve_export_output, resolve_format, resolve_output, write_output,
};
pub use render::MAX_WARNINGS;
pub use strudeland_host::{
    ExportPreset, HostError, LengthRequest, MAX_TIMEOUT_SEC, timeout_from_secs,
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
    render_audio_for(code, opts, None, progress).map(|(audio, _)| audio)
}

struct LoopAudio {
    start_bar: u32,
    isolated_body: Option<Vec<Vec<f32>>>,
}

fn render_audio_for(
    code: &str,
    opts: &RenderOptions,
    preset: Option<ExportPreset>,
    progress: &dyn Fn(Progress),
) -> Result<(RenderedAudio, Option<LoopAudio>), RenderError> {
    let started = Instant::now();
    render::validate_jobs(opts.jobs)?;
    let (planned, plan_ms) = plan::plan(code, opts, preset)?;
    let chunk_count = planned.chunks.len()
        + planned
            .export
            .as_ref()
            .and_then(|export| export.repeat.as_ref())
            .map_or(0, |repeat| repeat.chunks.len());
    progress(Progress::Planned {
        chunks: chunk_count,
    });
    let mut jobs = plan::check(&planned)?;
    let PlannedRender {
        window,
        cps,
        structure,
        lookback_sec,
        errors: plan_errors,
        events,
        chunks,
        export,
    } = planned;
    let tail_frames = export.as_ref().map_or(0, |export| export.tail_frames);
    let loop_start_bar = export.as_ref().and_then(|export| export.loop_start_bar);
    let repeat = export.and_then(|export| export.repeat);
    let repeat_jobs = jobs.split_off(chunks.len());
    let chunks: Vec<ChunkWindow> = chunks.into_iter().map(|c| c.plan).collect();
    let workers = render::workers_for(opts.jobs, chunks.len());

    let render_started = Instant::now();
    let Rendered {
        channels,
        errors: mut engine_errors,
    } = render_window(
        &window,
        cps,
        &chunks,
        tail_frames,
        jobs,
        workers,
        &|event| report_chunk(progress, event, 0, chunk_count),
    )?;
    let (isolated_body, repeat_workers) = if let Some(repeat) = repeat {
        let from_bar = loop_start_bar
            .ok_or_else(|| RenderError::Render("the isolated body has no loop start".into()))?;
        let body_window = PlannedWindow { from_bar, ..window };
        let body_chunks: Vec<_> = repeat.chunks.into_iter().map(|chunk| chunk.plan).collect();
        let repeat_workers = render::workers_for(opts.jobs, body_chunks.len());
        let body = render_window(
            &body_window,
            cps,
            &body_chunks,
            repeat.tail_frames,
            repeat_jobs,
            repeat_workers,
            &|event| report_chunk(progress, event, chunks.len(), chunk_count),
        )?;
        engine_errors.extend(body.errors);
        (Some(body.channels), repeat_workers)
    } else {
        (None, 0)
    };
    let render_ms = ms_since(render_started);

    let frames = join::window_frames(window.from_bar, window.to_bar, cps)
        .checked_add(tail_frames)
        .ok_or_else(|| RenderError::Render("joined frame count overflows".into()))?;
    let summary = Summary {
        format: None,
        bytes: None,
        window,
        frames,
        seconds: frames as f64 / f64::from(SAMPLE_RATE),
        sample_rate: SAMPLE_RATE,
        chunks: chunk_count,
        jobs: workers.max(repeat_workers),
        loudness: None,
        timings: Timings {
            plan_ms,
            render_ms,
            level_ms: 0.0,
            encode_ms: 0.0,
            total_ms: ms_since(started),
        },
        warnings: render::warnings(&plan_errors, &engine_errors),
    };
    Ok((
        RenderedAudio {
            channels,
            cps,
            structure,
            lookback_sec,
            chunks,
            events,
            plan_errors,
            summary,
        },
        loop_start_bar.map(|start_bar| LoopAudio {
            start_bar,
            isolated_body,
        }),
    ))
}

fn report_chunk(progress: &dyn Fn(Progress), event: Progress, offset: usize, count: usize) {
    if let Progress::ChunkRendered { index, .. } = event {
        progress(Progress::ChunkRendered {
            index: offset + index,
            of: count,
        });
    }
}

fn render_window(
    window: &PlannedWindow,
    cps: f64,
    chunks: &[ChunkWindow],
    tail_frames: u64,
    jobs: Vec<EngineJob>,
    workers: usize,
    progress: &dyn Fn(Progress),
) -> Result<Rendered, RenderError> {
    let mut joiner = join::Joiner::for_plans_with_tail(window, cps, chunks, 2, tail_frames)?;
    let errors = render::render_chunks(
        jobs,
        chunks,
        workers,
        strudeland_engine::render,
        &mut joiner,
        progress,
    )?;
    Ok(Rendered {
        channels: joiner.finish()?,
        errors,
    })
}

/// Steps 5 and 6: level and encode a joined buffer, verifying and correcting a lossy file's
/// decoded true peak ([`encode::encode`]). Callable several times on one [`RenderedAudio`]. The
/// summary starts from `audio.summary` and gains the format, size, loudness and timings; level
/// and encoder warnings follow the render's.
pub fn finish(
    audio: &RenderedAudio,
    out: &OutputOptions,
    progress: &dyn Fn(Progress),
) -> Result<RenderOutput, RenderError> {
    finish_file(audio, out, progress, false)
}

fn finish_file(
    audio: &RenderedAudio,
    out: &OutputOptions,
    progress: &dyn Fn(Progress),
    standalone: bool,
) -> Result<RenderOutput, RenderError> {
    let mut summary = audio.summary.clone();
    let mut warnings = Vec::new();
    let encode = if standalone {
        encode::encode_standalone
    } else {
        encode::encode
    };
    let encoded = encode(
        &audio.channels,
        audio.summary.sample_rate,
        out,
        progress,
        &mut warnings,
    )?;
    summary.format = Some(out.encoder.format());
    summary.bytes = Some(encoded.bytes.len() as u64);
    summary.loudness = Some(encoded.loudness);
    summary.timings.level_ms = encoded.level_ms;
    summary.timings.encode_ms = encoded.encode_ms;
    summary.timings.total_ms += encoded.level_ms + encoded.encode_ms;
    summary.warnings.extend(warnings);
    Ok(RenderOutput {
        bytes: encoded.bytes,
        summary,
    })
}

fn ms_since(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}
