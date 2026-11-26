//! The pipeline after planning: decode and refuse the jobs, render the opening and the isolated
//! body through a [`ChunkRenderer`], join them and build the summary.

use std::time::Instant;

use strudeland_engine::{EngineJob, Rendered, StemLayout};
use strudeland_host::{ChunkWindow, PlannedRender, PlannedWindow, SAMPLE_RATE};

use crate::{
    AudioMode, LoopAudio, PipelineAudio, Progress, RenderError, RenderOptions, RenderedAudio,
    Summary, Timings, join, ms_since, plan, render, stems,
};

/// Renders one chunk job. The real one is the engine; tests inject synthetic PCM.
pub(crate) trait ChunkRenderer: Sync {
    fn render(
        &self,
        job: &EngineJob,
        stems: Option<&StemLayout>,
    ) -> Result<Rendered, strudeland_engine::JobError>;
}

/// The engine: [`strudeland_engine::render_with_stems`] with a layout, else
/// [`strudeland_engine::render`].
pub(crate) struct EngineChunks;

impl ChunkRenderer for EngineChunks {
    fn render(
        &self,
        job: &EngineJob,
        stems: Option<&StemLayout>,
    ) -> Result<Rendered, strudeland_engine::JobError> {
        match stems {
            Some(layout) => strudeland_engine::render_with_stems(job, layout),
            None => strudeland_engine::render(job),
        }
    }
}

/// When the render started (the summary's `total_ms` runs from it) and how long planning took.
pub(crate) struct PlanTiming {
    pub started: Instant,
    pub plan_ms: f64,
}

/// Steps 2 to 4 on a finished plan: decode the jobs and refuse unsupported material, render the
/// opening (with `mode`'s stem layout) and any isolated body (without one) through `renderer`,
/// join each pass and build the summary.
pub(crate) fn render_planned(
    mut planned: PlannedRender,
    timing: PlanTiming,
    opts: &RenderOptions,
    mode: AudioMode,
    renderer: &dyn ChunkRenderer,
    progress: &dyn Fn(Progress),
) -> Result<PipelineAudio, RenderError> {
    let PlanTiming { started, plan_ms } = timing;
    let chunk_count = planned.chunks.len()
        + planned
            .export
            .as_ref()
            .and_then(|export| export.repeat.as_ref())
            .map_or(0, |repeat| repeat.chunks.len());
    progress(Progress::Planned {
        chunks: chunk_count,
    });
    let mut jobs = plan::take_jobs(&mut planned)?;
    let PlannedRender {
        window,
        music_frames,
        cps,
        structure,
        lookback_sec,
        errors: plan_errors,
        events,
        chunks,
        export,
    } = planned;
    let tail_frames = export.as_ref().map_or(0, |export| export.tail_frames);
    let music_frames =
        music_frames.unwrap_or_else(|| join::window_frames(window.from_bar, window.to_bar, cps));
    let loop_start = export.as_ref().and_then(|export| {
        let bar = export.loop_start_bar?;
        let frame = export
            .loop_start_frame
            .unwrap_or_else(|| join::window_frames(window.from_bar, bar, cps));
        Some((bar, frame))
    });
    let repeat_jobs = jobs.split_off(chunks.len());
    let chunks: Vec<ChunkWindow> = chunks.into_iter().map(|c| c.plan).collect();
    let workers = render::workers_for(opts.jobs, chunks.len());
    let frames = music_frames
        .checked_add(tail_frames)
        .ok_or_else(|| RenderError::render("joined frame count overflows"))?;
    let (stems, channels) = if let AudioMode::Stems(grouping) = mode {
        let layout = StemLayout::for_jobs(&jobs, grouping.into());
        let channels = stems::validate_budget(layout.ids().len(), frames, &chunks, workers)?;
        (Some(layout), channels)
    } else {
        (None, 2)
    };

    let render_started = Instant::now();
    let Rendered {
        channels: joined,
        errors: mut engine_errors,
    } = render_window(
        join::Joiner::for_plans_with_tail(
            &window,
            cps,
            &chunks,
            channels,
            Some(music_frames),
            tail_frames,
        )?,
        &chunks,
        jobs,
        workers,
        |job| renderer.render(job, stems.as_ref()),
        &|event| report_chunk(progress, event, 0, chunk_count),
    )?;
    let (isolated_body, repeat_workers) =
        if let Some(repeat) = export.and_then(|export| export.repeat) {
            let (from_bar, start_frame) = loop_start
                .ok_or_else(|| RenderError::render("the isolated body has no loop start"))?;
            let expected = music_frames
                .checked_sub(start_frame)
                .ok_or_else(|| RenderError::render("loop begins past the opening"))?;
            if repeat.music_frames.is_some_and(|frames| frames != expected) {
                return Err(RenderError::render(
                    "isolated body length differs from the opening loop",
                ));
            }
            let body_window = PlannedWindow { from_bar, ..window };
            let body_chunks: Vec<_> = repeat.chunks.into_iter().map(|chunk| chunk.plan).collect();
            let repeat_workers = render::workers_for(opts.jobs, body_chunks.len());
            let body = render_window(
                join::Joiner::for_plans_with_tail(
                    &body_window,
                    cps,
                    &body_chunks,
                    2,
                    Some(expected),
                    repeat.tail_frames,
                )?,
                &body_chunks,
                repeat_jobs,
                repeat_workers,
                |job| renderer.render(job, None),
                &|event| report_chunk(progress, event, chunks.len(), chunk_count),
            )?;
            engine_errors.extend(body.errors);
            (Some(body.channels), repeat_workers)
        } else {
            (None, 0)
        };
    let render_ms = ms_since(render_started);

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
    Ok(PipelineAudio {
        audio: RenderedAudio {
            channels: joined,
            music_frames,
            cps,
            structure,
            lookback_sec,
            chunks,
            events,
            plan_errors,
            summary,
        },
        loop_audio: loop_start.map(|(start_bar, start_frame)| LoopAudio {
            start_bar,
            start_frame,
            isolated_body,
        }),
        stems,
    })
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
    mut joiner: join::Joiner,
    chunks: &[ChunkWindow],
    jobs: Vec<EngineJob>,
    workers: usize,
    render: impl Fn(&EngineJob) -> Result<Rendered, strudeland_engine::JobError> + Sync,
    progress: &dyn Fn(Progress),
) -> Result<Rendered, RenderError> {
    let errors = render::render_chunks(jobs, chunks, workers, render, &mut joiner, progress)?;
    Ok(Rendered {
        channels: joiner.finish()?,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::{Value, json};
    use strudeland_engine::{ErrorKind, RenderError as EngineRenderError};
    use strudeland_host::{
        Layout, PlanError, PlannedChunk, PlannedExport, PlannedRepeat, PlannedWindow,
    };

    use super::*;
    use crate::{StemGrouping, bench, testing};

    const CPS: f64 = 0.5;

    /// The minimal engine job: `frames` frames, `seed` (the [`Ramp`]'s chunk marker) and one hap
    /// on each of `values`.
    fn job(frames: u64, seed: i32, values: &[Value]) -> String {
        let haps: Vec<Value> = (0..values.len())
            .map(|v| json!({ "t": 0.0, "duration": 0.25, "cycle": 0.0, "v": v, "event": v }))
            .collect();
        // The validator wants at least one pause.
        let pauses = json!([{ "suspendSec": 0.0, "haps": haps }]);
        let text = json!({
            "schema": 2, "sampleRate": 44100, "frames": frames, "seed": seed, "cps": 2,
            "channels": { "count": 2, "multiChannelOrbits": false },
            "orbits": [{ "orbit": 1, "channels": [0, 1] }],
            "pauses": pauses,
            "values": values,
        })
        .to_string();
        EngineJob::from_json(&text).unwrap();
        text
    }

    /// A chunk over bars `[from_bar, to_bar)` keeping `content` frames after a `head` of
    /// lookback, with `extra` frames (a tail) after them; its job carries `seed` and `values`.
    fn chunk(
        from_bar: u32,
        to_bar: u32,
        head: u64,
        content: u64,
        extra: u64,
        seed: i32,
        values: &[Value],
    ) -> PlannedChunk {
        let frames = head + content + extra;
        PlannedChunk {
            json: job(frames, seed, values),
            plan: ChunkWindow {
                c0: from_bar,
                c1: to_bar,
                lookback_sec_used: 0.0,
                window_start_frame: head,
                content_frames: Some(content),
                buffer_start_sec: 0.0,
                frames,
                from_bar,
                to_bar,
                lookback_sec: 0.0,
            },
        }
    }

    fn planned(
        window: (u32, u32),
        music_frames: Option<u64>,
        chunks: Vec<PlannedChunk>,
        export: Option<PlannedExport>,
    ) -> PlannedRender {
        PlannedRender {
            window: PlannedWindow {
                from_bar: window.0,
                to_bar: window.1,
                layout: Layout::Clip,
            },
            export,
            cps: CPS,
            music_frames,
            structure: testing::structure(),
            lookback_sec: 0.0,
            errors: vec![],
            events: None,
            chunks,
        }
    }

    /// Sample `i` of chunk `marker`'s ramp.
    fn at(marker: i32, i: usize) -> f32 {
        marker as f32 + i as f32 * 1e-6
    }

    fn ramp(marker: i32, range: std::ops::Range<usize>) -> Vec<f32> {
        range.map(|i| at(marker, i)).collect()
    }

    /// Returns each job's ramp on every channel and records the layout it was given.
    #[derive(Default)]
    struct Ramp {
        calls: AtomicUsize,
        /// `(marker, stem count)` per call; `None` when no layout was passed.
        seen: Mutex<Vec<(i32, Option<usize>)>>,
        errors: Vec<EngineRenderError>,
    }

    impl ChunkRenderer for Ramp {
        fn render(
            &self,
            job: &EngineJob,
            stems: Option<&StemLayout>,
        ) -> Result<Rendered, strudeland_engine::JobError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let stem_count = stems.map(|layout| layout.ids().len());
            self.seen.lock().unwrap().push((job.seed, stem_count));
            let channels = stem_count.map_or(2, |n| n * 2 + 2);
            let frames = job.frames as usize;
            Ok(Rendered {
                channels: vec![ramp(job.seed, 0..frames); channels],
                errors: self.errors.clone(),
            })
        }
    }

    impl Ramp {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    fn run(
        plan: PlannedRender,
        jobs: usize,
        mode: AudioMode,
        renderer: &Ramp,
    ) -> (Result<PipelineAudio, RenderError>, Vec<Progress>) {
        let events = RefCell::new(Vec::new());
        let result = render_planned(
            plan,
            PlanTiming {
                started: Instant::now(),
                plan_ms: 7.0,
            },
            &bench::render_options(2, jobs),
            mode,
            renderer,
            &|event| events.borrow_mut().push(event),
        );
        (result, events.into_inner())
    }

    fn render_err(result: Result<PipelineAudio, RenderError>) -> String {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(e) => e.to_string(),
        }
    }

    fn rendered(index: usize, of: usize) -> Progress {
        Progress::ChunkRendered { index, of }
    }

    #[test]
    fn a_plan_renders_and_joins_its_chunks() {
        let plan = planned(
            (0, 2),
            Some(2000),
            vec![
                chunk(0, 1, 10, 1000, 0, 1, &[]),
                chunk(1, 2, 20, 1000, 0, 2, &[]),
            ],
            None,
        );
        let ramp_renderer = Ramp::default();
        let (result, mut events) = run(plan, 4, AudioMode::Standard, &ramp_renderer);
        let out = result.unwrap();
        let mut expected = ramp(1, 10..1010);
        expected.extend(ramp(2, 20..1020));
        assert_eq!(out.audio.channels, vec![expected.clone(), expected]);
        assert_eq!(out.audio.music_frames, 2000);
        assert_eq!(out.audio.summary.frames, 2000);
        assert_eq!(out.audio.summary.chunks, 2);
        assert_eq!(out.audio.summary.jobs, 2);
        assert_eq!(out.audio.summary.timings.plan_ms, 7.0);
        assert_eq!(out.audio.chunks.len(), 2);
        assert!(out.loop_audio.is_none());
        assert!(out.stems.is_none());
        assert_eq!(ramp_renderer.calls(), 2);
        assert_eq!(events.first(), Some(&Progress::Planned { chunks: 2 }));
        // Two workers report in completion order.
        events[1..].sort_by_key(|e| match e {
            Progress::ChunkRendered { index, .. } => *index,
            _ => usize::MAX,
        });
        assert_eq!(
            events,
            [
                Progress::Planned { chunks: 2 },
                rendered(0, 2),
                rendered(1, 2)
            ]
        );
    }

    #[test]
    fn unsupported_material_is_refused_before_any_render() {
        let plan = planned(
            (0, 1),
            Some(1000),
            vec![chunk(0, 1, 0, 1000, 0, 1, &[json!({ "s": "bd" })])],
            None,
        );
        let ramp_renderer = Ramp::default();
        let (result, events) = run(plan, 1, AudioMode::Standard, &ramp_renderer);
        match result {
            Err(RenderError::Unsupported(list)) => {
                assert_eq!(list.len(), 1);
                assert_eq!(list[0].s, "bd");
            }
            Err(e) => panic!("expected Unsupported, got {e}"),
            Ok(_) => panic!("expected Unsupported"),
        }
        assert_eq!(ramp_renderer.calls(), 0);
        assert_eq!(events, [Progress::Planned { chunks: 1 }]);
    }

    #[test]
    fn a_tail_extends_the_window() {
        let export = PlannedExport {
            loop_start_bar: None,
            loop_start_frame: None,
            tail_frames: 100,
            repeat: None,
        };
        let plan = planned(
            (0, 2),
            Some(2000),
            vec![
                chunk(0, 1, 0, 1000, 0, 1, &[]),
                chunk(1, 2, 5, 1000, 100, 2, &[]),
            ],
            Some(export),
        );
        let (result, _) = run(plan, 1, AudioMode::Standard, &Ramp::default());
        let out = result.unwrap();
        let mut expected = ramp(1, 0..1000);
        expected.extend(ramp(2, 5..1105));
        assert_eq!(out.audio.channels[0].len(), 2000 + 100);
        assert_eq!(out.audio.channels[0], expected);
        assert_eq!(out.audio.music_frames, 2000);
        assert_eq!(out.audio.summary.frames, 2000 + 100);
        assert!(out.loop_audio.is_none());
    }

    #[test]
    fn missing_frames_fall_back_to_the_window() {
        let bar = join::window_frames(0, 1, CPS);
        assert_eq!(bar, 88_200);
        let export = PlannedExport {
            loop_start_bar: Some(1),
            loop_start_frame: None,
            tail_frames: 0,
            repeat: None,
        };
        let plan = planned(
            (0, 2),
            None,
            vec![
                chunk(0, 1, 0, bar, 0, 1, &[]),
                chunk(1, 2, 0, bar, 0, 2, &[]),
            ],
            Some(export),
        );
        let (result, _) = run(plan, 1, AudioMode::Standard, &Ramp::default());
        let out = result.unwrap();
        assert_eq!(out.audio.music_frames, join::window_frames(0, 2, CPS));
        assert_eq!(out.audio.summary.frames, 2 * bar);
        let loop_audio = out.loop_audio.unwrap();
        assert_eq!(loop_audio.start_bar, 1);
        assert_eq!(loop_audio.start_frame, join::window_frames(0, 1, CPS));
        assert!(loop_audio.isolated_body.is_none());
    }

    fn isolated_export(
        loop_start_bar: Option<u32>,
        loop_start_frame: Option<u64>,
        repeat_music: Option<u64>,
    ) -> PlannedExport {
        PlannedExport {
            loop_start_bar,
            loop_start_frame,
            tail_frames: 0,
            repeat: Some(PlannedRepeat {
                music_frames: repeat_music,
                tail_frames: 50,
                chunks: vec![chunk(1, 2, 10, 1000, 50, 3, &[])],
            }),
        }
    }

    fn opening(export: PlannedExport) -> PlannedRender {
        planned(
            (0, 2),
            Some(2000),
            vec![
                chunk(0, 1, 0, 1000, 0, 1, &[]),
                chunk(1, 2, 0, 1000, 0, 2, &[]),
            ],
            Some(export),
        )
    }

    #[test]
    fn the_isolated_body_renders_after_the_opening() {
        let plan = opening(isolated_export(Some(1), Some(1000), Some(1000)));
        let ramp_renderer = Ramp::default();
        let (result, mut events) = run(plan, 4, AudioMode::Standard, &ramp_renderer);
        let out = result.unwrap();
        let mut seen = ramp_renderer.seen.lock().unwrap().clone();
        assert_eq!(seen.last(), Some(&(3, None)), "the repeat renders last");
        seen.sort();
        assert_eq!(seen, [(1, None), (2, None), (3, None)]);

        assert_eq!(events.first(), Some(&Progress::Planned { chunks: 3 }));
        assert_eq!(
            events.last(),
            Some(&rendered(2, 3)),
            "the repeat's index is offset"
        );
        events[1..3].sort_by_key(|e| match e {
            Progress::ChunkRendered { index, .. } => *index,
            _ => usize::MAX,
        });
        assert_eq!(
            events,
            [
                Progress::Planned { chunks: 3 },
                rendered(0, 3),
                rendered(1, 3),
                rendered(2, 3)
            ]
        );

        let loop_audio = out.loop_audio.unwrap();
        assert_eq!(loop_audio.start_bar, 1);
        assert_eq!(loop_audio.start_frame, 1000);
        let body = ramp(3, 10..1060);
        assert_eq!(loop_audio.isolated_body, Some(vec![body.clone(), body]));
        assert_eq!(out.audio.summary.chunks, 3);
        // The opening's two workers, not the repeat's one.
        assert_eq!(out.audio.summary.jobs, 2);
        assert_eq!(out.audio.summary.frames, 2000);
        assert_eq!(out.audio.chunks.len(), 2);
    }

    #[test]
    fn isolated_body_errors() {
        let cases = [
            (
                isolated_export(None, None, Some(1000)),
                "render failed: the isolated body has no loop start",
            ),
            (
                isolated_export(Some(1), Some(3000), None),
                "render failed: loop begins past the opening",
            ),
            (
                isolated_export(Some(1), Some(1000), Some(999)),
                "render failed: isolated body length differs from the opening loop",
            ),
        ];
        for (export, text) in cases {
            let ramp_renderer = Ramp::default();
            let (result, _) = run(opening(export), 1, AudioMode::Standard, &ramp_renderer);
            assert_eq!(render_err(result), text);
            assert_eq!(ramp_renderer.calls(), 2, "{text}: the opening rendered");
        }
    }

    #[test]
    fn stems_pass_the_layout_and_respect_the_budget() {
        let sine = [json!({ "s": "sine" })];
        let plan = planned(
            (0, 2),
            Some(2000),
            vec![
                chunk(0, 1, 0, 1000, 0, 1, &sine),
                chunk(1, 2, 0, 1000, 0, 2, &sine),
            ],
            None,
        );
        let ramp_renderer = Ramp::default();
        let mode = AudioMode::Stems(StemGrouping::Orbit);
        let (result, _) = run(plan, 2, mode, &ramp_renderer);
        let out = result.unwrap();
        let layout = out.stems.unwrap();
        assert_eq!(layout.ids().len(), 1);
        let seen = ramp_renderer.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|(_, stems)| *stems == Some(1)), "{seen:?}");
        assert_eq!(out.audio.channels.len(), 4);

        for (frames, text) in [
            (
                1u64 << 29,
                "encode failed: a stem WAV would exceed the RIFF size limit",
            ),
            (
                1u64 << 26,
                "render failed: stem audio exceeds the 512 MiB buffer limit; select a shorter \
                 range or use fewer workers",
            ),
        ] {
            let plan = planned(
                (0, 1),
                Some(frames),
                vec![chunk(0, 1, 0, 1000, 0, 1, &sine)],
                None,
            );
            let ramp_renderer = Ramp::default();
            let mode = AudioMode::Stems(StemGrouping::Orbit);
            let (result, _) = run(plan, 1, mode, &ramp_renderer);
            assert_eq!(render_err(result), text);
            assert_eq!(ramp_renderer.calls(), 0);
        }
    }

    #[test]
    fn engine_errors_become_render_warnings() {
        let mut plan = planned(
            (0, 1),
            Some(1000),
            vec![chunk(0, 1, 0, 1000, 0, 1, &[])],
            None,
        );
        plan.errors = vec![PlanError {
            kind: "eval".into(),
            message: "bad mini".into(),
            bar: Some(0),
        }];
        let ramp_renderer = Ramp {
            errors: vec![EngineRenderError::new(ErrorKind::Hap, "too loud", Some(2))],
            ..Ramp::default()
        };
        let (result, _) = run(plan, 1, AudioMode::Standard, &ramp_renderer);
        let out = result.unwrap();
        assert_eq!(
            out.audio.summary.warnings,
            [
                "plan: eval: bad mini (bar 1)",
                "render: hap: too loud (bar 3)"
            ]
        );
        assert_eq!(out.audio.plan_errors.len(), 1);
    }
}
