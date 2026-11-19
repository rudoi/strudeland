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
pub mod join;
pub mod level;
mod options;
pub mod output;
mod plan;
mod render;
mod summary;

use std::time::Instant;

use strudeland_host::{ChunkWindow, PlannedRender, SAMPLE_RATE};

pub use error::{ExitClass, RenderError, Unsupported};
pub use options::{
    BitDepth, DEFAULT_MAX_JOBS, DEFAULT_TARGET_LUFS, DEFAULT_TRUE_PEAK_DB, EncoderOptions, Format,
    Loudness, Mp3Mode, OnOff, OutputOptions, RenderOptions,
};
pub use output::{OutputTarget, resolve_format, resolve_output, write_output};
pub use render::MAX_WARNINGS;
pub use strudeland_host::{HostError, LengthRequest, MAX_TIMEOUT_SEC, timeout_from_secs};
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
    let started = Instant::now();
    render::validate_jobs(opts.jobs)?;
    let (planned, plan_ms) = plan::plan(code, opts)?;
    progress(Progress::Planned {
        chunks: planned.chunks.len(),
    });
    let jobs = plan::check(&planned)?;
    let PlannedRender {
        window,
        cps,
        structure,
        lookback_sec,
        errors: plan_errors,
        events,
        chunks,
    } = planned;
    let chunks: Vec<ChunkWindow> = chunks.into_iter().map(|c| c.plan).collect();
    let workers = render::workers_for(opts.jobs, chunks.len());

    let render_started = Instant::now();
    // The window is allocated before the renders and each chunk's content is copied in as it
    // arrives, so the joined song and the chunks in flight are all that is held at once.
    let mut joiner = join::Joiner::for_plans(&window, cps, &chunks, 2)?;
    let engine_errors = render::render_chunks(
        jobs,
        &chunks,
        workers,
        strudeland_engine::render,
        &mut joiner,
        progress,
    )?;
    let channels = joiner.finish()?;
    let render_ms = ms_since(render_started);

    let frames = join::window_frames(window.from_bar, window.to_bar, cps);
    let summary = Summary {
        format: None,
        bytes: None,
        window,
        frames,
        seconds: frames as f64 / f64::from(SAMPLE_RATE),
        sample_rate: SAMPLE_RATE,
        chunks: chunks.len(),
        jobs: workers,
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
    Ok(RenderedAudio {
        channels,
        cps,
        structure,
        lookback_sec,
        chunks,
        events,
        plan_errors,
        summary,
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
    let mut summary = audio.summary.clone();
    let mut warnings = Vec::new();
    let encoded = encode::encode(
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
