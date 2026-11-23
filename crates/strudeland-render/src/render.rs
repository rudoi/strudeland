//! Rendering the chunk jobs on a scoped worker pool, the failure rules and the warnings.

use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};

use strudeland_engine::{ErrorKind, JobError, RenderError as EngineRenderError, Rendered};
use strudeland_host::{ChunkWindow, PlanError};

use crate::join::Joiner;
use crate::options::DEFAULT_MAX_JOBS;
use crate::{Progress, RenderError};

/// Warnings listed per source (planner, engine) before the rest are counted.
pub const MAX_WARNINGS: usize = 20;

/// A worker thread's stack, as large as a main thread's.
const WORKER_STACK: usize = 8 << 20;

/// `Some(0)` is a usage error; everything else is accepted (clamping comes later).
pub(crate) fn validate_jobs(jobs: Option<usize>) -> Result<(), RenderError> {
    if jobs == Some(0) {
        return Err(RenderError::Usage {
            flag: "--jobs",
            message: "--jobs must be at least 1".into(),
        });
    }
    Ok(())
}

/// The worker count: `requested`, else `min(available, 4)` (`available` unknown counts as 1),
/// clamped to `chunks` and at least 1.
pub(crate) fn worker_count(
    requested: Option<usize>,
    available: Option<NonZeroUsize>,
    chunks: usize,
) -> usize {
    let wanted =
        requested.unwrap_or_else(|| available.map_or(1, NonZeroUsize::get).min(DEFAULT_MAX_JOBS));
    wanted.min(chunks).max(1)
}

/// [`worker_count`] with this machine's parallelism.
pub(crate) fn workers_for(requested: Option<usize>, chunks: usize) -> usize {
    worker_count(requested, std::thread::available_parallelism().ok(), chunks)
}

/// One chunk's outcome as a worker reports it: the rendered channels, whole.
type Outcome = (
    usize,
    Result<Vec<Vec<f32>>, RenderError>,
    Vec<EngineRenderError>,
);

/// Renders `jobs` with `workers` threads. `render` renders one job; each result is checked
/// against the output layout and `plan.frames` on the worker and sent to the calling thread,
/// which copies its content into `joiner` and frees the buffer, and alone calls `progress`
/// (`ChunkRendered`, in completion order). So at most the window and the chunks in flight are
/// held at once. The first failure stops workers from starting new chunks; the error of the
/// lowest failing chunk index is returned. On success: the engine's non-fatal render errors in
/// chunk order.
pub(crate) fn render_chunks<J, F>(
    jobs: Vec<J>,
    plans: &[ChunkWindow],
    workers: usize,
    render: F,
    joiner: &mut Joiner,
    progress: &dyn Fn(Progress),
) -> Result<Vec<EngineRenderError>, RenderError>
where
    J: Send,
    F: Fn(&J) -> Result<Rendered, JobError> + Sync,
{
    let of = jobs.len();
    if plans.len() != of {
        return Err(RenderError::Render(format!(
            "{of} jobs for {} chunk plans",
            plans.len()
        )));
    }
    // Each worker takes its job out of the slot, so a job is freed once rendered.
    let slots: Vec<Mutex<Option<J>>> = jobs.into_iter().map(|j| Mutex::new(Some(j))).collect();
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let mut results: Vec<Option<Vec<EngineRenderError>>> = (0..of).map(|_| None).collect();
    let mut failure: Option<(usize, RenderError)> = None;
    let channels = joiner.channels();

    std::thread::scope(|scope| -> Result<(), RenderError> {
        // Hand each result directly to the joiner, keeping at most one buffer per
        // worker plus the buffer being placed by the calling thread.
        let (sender, inbox) = sync_channel::<Outcome>(0);
        let mut spawned = 0;
        let mut spawn_error = None;
        for w in 0..workers.max(1) {
            let sender = sender.clone();
            let (slots, next, stop, render) = (&slots, &next, &stop, &render);
            let spawn = std::thread::Builder::new()
                .name(format!("strudeland-render-{w}"))
                .stack_size(WORKER_STACK)
                .spawn_scoped(scope, move || {
                    worker(slots, plans, channels, next, stop, render, &sender)
                });
            match spawn {
                Ok(_) => spawned += 1,
                Err(e) => {
                    spawn_error = Some(e);
                    break;
                }
            }
        }
        drop(sender);
        if spawned == 0 {
            stop.store(true, Ordering::SeqCst);
            let e = spawn_error.map_or_else(|| "no worker".to_string(), |e| e.to_string());
            return Err(RenderError::Render(format!(
                "starting a render thread: {e}"
            )));
        }
        // The workers' senders close when they finish, which ends this loop.
        for (index, result, errors) in inbox {
            let placed = result.and_then(|channels| {
                let plan = plans
                    .get(index)
                    .ok_or_else(|| RenderError::Render(format!("chunk {index}: no plan")))?;
                joiner.place_rendered(index, plan, &channels)
                // `channels`, the whole chunk buffer, is freed here.
            });
            match placed {
                Ok(()) => {
                    if let Some(slot) = results.get_mut(index) {
                        *slot = Some(errors);
                    }
                    progress(Progress::ChunkRendered { index, of });
                }
                Err(e) => {
                    stop.store(true, Ordering::SeqCst);
                    if failure.as_ref().is_none_or(|(at, _)| index < *at) {
                        failure = Some((index, e));
                    }
                }
            }
        }
        Ok(())
    })?;

    if let Some((_, e)) = failure {
        return Err(e);
    }
    let mut warnings = Vec::new();
    for (i, r) in results.into_iter().enumerate() {
        let Some(errors) = r else {
            return Err(RenderError::Render(format!("chunk {i}: not rendered")));
        };
        warnings.extend(errors);
    }
    Ok(warnings)
}

/// A worker: takes chunk indices until none is left or a failure stops the pool.
fn worker<J, F>(
    slots: &[Mutex<Option<J>>],
    plans: &[ChunkWindow],
    channels: usize,
    next: &AtomicUsize,
    stop: &AtomicBool,
    render: &F,
    sender: &SyncSender<Outcome>,
) where
    F: Fn(&J) -> Result<Rendered, JobError>,
{
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let index = next.fetch_add(1, Ordering::SeqCst);
        let (Some(slot), Some(plan)) = (slots.get(index), plans.get(index)) else {
            return;
        };
        let job = match slot.lock() {
            Ok(mut guard) => guard.take(),
            Err(_) => None,
        };
        let outcome = match job {
            Some(job) => render_one(index, plan, channels, render(&job)),
            None => (
                Err(RenderError::Render(format!("chunk {index}: job missing"))),
                Vec::new(),
            ),
        };
        if outcome.0.is_err() {
            stop.store(true, Ordering::SeqCst);
        }
        if sender.send((index, outcome.0, outcome.1)).is_err() {
            return;
        }
    }
}

/// The failure rules for one chunk's render result.
fn render_one(
    index: usize,
    plan: &ChunkWindow,
    channels: usize,
    rendered: Result<Rendered, JobError>,
) -> (Result<Vec<Vec<f32>>, RenderError>, Vec<EngineRenderError>) {
    let fail = |m: String| {
        (
            Err(RenderError::Render(format!("chunk {index}: {m}"))),
            Vec::new(),
        )
    };
    let rendered = match rendered {
        Ok(r) => r,
        Err(e) => return fail(e.message),
    };
    if let Some(e) = rendered
        .errors
        .iter()
        .find(|e| e.kind == ErrorKind::Unsupported)
    {
        return fail(format!("unsupported: {}", e.message));
    }
    if rendered.channels.len() != channels {
        let expected = if channels == 2 {
            "a mix has 2".to_owned()
        } else {
            format!("the output layout has {channels}")
        };
        return fail(format!(
            "rendered {} channels, {expected}",
            rendered.channels.len()
        ));
    }
    if let Some(short) = rendered
        .channels
        .iter()
        .map(Vec::len)
        .find(|&n| n as u64 != plan.frames)
    {
        return fail(format!(
            "rendered {short} frames, the plan says {}",
            plan.frames
        ));
    }
    (Ok(rendered.channels), rendered.errors)
}

/// `(bar <bar + 1>)` when there is a bar.
fn bar_suffix(bar: Option<i64>) -> String {
    bar.map_or_else(String::new, |b| format!(" (bar {})", b.saturating_add(1)))
}

/// Deduplicates `lines` (first occurrence order) and keeps [`MAX_WARNINGS`], then one line
/// `<source>: and <n> more warnings`.
fn capped(source: &str, lines: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = lines
        .into_iter()
        .filter(|l| seen.insert(l.clone()))
        .collect();
    let more = unique.len().saturating_sub(MAX_WARNINGS);
    let mut out: Vec<String> = unique.into_iter().take(MAX_WARNINGS).collect();
    if more > 0 {
        out.push(format!("{source}: and {more} more warnings"));
    }
    out
}

/// The summary's warnings: the planner's errors, then the engine's non-fatal render errors.
pub(crate) fn warnings(plan: &[PlanError], render: &[EngineRenderError]) -> Vec<String> {
    let mut out = capped(
        "plan",
        plan.iter()
            .map(|e| format!("plan: {}: {}{}", e.kind, e.message, bar_suffix(e.bar))),
    );
    out.extend(capped(
        "render",
        render
            .iter()
            .map(|e| format!("render: {}: {}{}", e.kind, e.message, bar_suffix(e.bar))),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::AtomicUsize;
    use strudeland_host::{Layout, PlannedWindow};

    fn nz(n: usize) -> Option<NonZeroUsize> {
        NonZeroUsize::new(n)
    }

    #[test]
    fn worker_count_rules() {
        // Default: min(available, 4), clamped to the chunks.
        assert_eq!(worker_count(None, nz(8), 10), 4);
        assert_eq!(worker_count(None, nz(2), 10), 2);
        assert_eq!(worker_count(None, nz(4), 10), 4);
        assert_eq!(worker_count(None, None, 10), 1);
        assert_eq!(worker_count(None, nz(8), 3), 3);
        // Explicit: clamped to the chunks, not to the parallelism.
        assert_eq!(worker_count(Some(16), nz(2), 10), 10);
        assert_eq!(worker_count(Some(6), nz(2), 10), 6);
        assert_eq!(worker_count(Some(3), nz(8), 2), 2);
        assert_eq!(worker_count(Some(1), nz(8), 5), 1);
        // Never 0, even without chunks.
        assert_eq!(worker_count(Some(4), nz(8), 0), 1);
        assert_eq!(worker_count(None, nz(8), 0), 1);
    }

    #[test]
    fn zero_jobs_is_a_usage_error() {
        let e = validate_jobs(Some(0)).unwrap_err();
        assert!(
            matches!(e, RenderError::Usage { flag: "--jobs", .. }),
            "{e:?}"
        );
        assert_eq!(e.to_string(), "--jobs must be at least 1");
        assert!(validate_jobs(Some(1)).is_ok());
        assert!(validate_jobs(None).is_ok());
    }

    /// A fake job: the chunk index, what to return and how long to take.
    struct Fake {
        index: usize,
        result: Result<Vec<EngineRenderError>, String>,
        channels: usize,
        frames: usize,
        delay_ms: u64,
    }

    fn ok(index: usize) -> Fake {
        Fake {
            index,
            result: Ok(vec![]),
            channels: 2,
            frames: 3,
            delay_ms: 0,
        }
    }

    /// Chunk `i` covers bar `i` (one frame per bar at cps 44100) with one frame of lookback and
    /// one of overlap: 3 frames.
    fn plans(n: usize) -> Vec<ChunkWindow> {
        (0..n)
            .map(|i| ChunkWindow {
                c0: i as u32,
                c1: i as u32 + 1,
                lookback_sec_used: 0.0,
                content_frames: None,
                window_start_frame: 1,
                buffer_start_sec: 0.0,
                frames: 3,
                from_bar: i as u32,
                to_bar: i as u32 + 1,
                lookback_sec: 0.0,
            })
            .collect()
    }

    fn fake_render(job: &Fake) -> Result<Rendered, JobError> {
        std::thread::sleep(std::time::Duration::from_millis(job.delay_ms));
        let errors = job.result.clone().map_err(JobError::new)?;
        let v = job.index as f32;
        // Lookback frame -1, the content frame, overlap frame -2.
        Ok(Rendered {
            channels: (0..job.channels)
                .map(|c| {
                    let mut ch = vec![-1.0, v + 10.0 * c as f32, -2.0];
                    ch.resize(job.frames, 0.0);
                    ch
                })
                .collect(),
            errors,
        })
    }

    type Pooled = Result<(Vec<Vec<f32>>, Vec<EngineRenderError>), RenderError>;

    /// A joiner for `plans(n)`: the window is bars `[0, n)`, one frame each.
    fn joiner(n: usize) -> Joiner {
        let window = PlannedWindow {
            from_bar: 0,
            to_bar: n as u32,
            layout: Layout::Linear,
        };
        Joiner::for_plans(&window, 44_100.0, &plans(n), 2).unwrap()
    }

    fn render_with<F>(jobs: Vec<Fake>, plans: &[ChunkWindow], workers: usize, render: F) -> Pooled
    where
        F: Fn(&Fake) -> Result<Rendered, JobError> + Sync,
    {
        let mut j = joiner(plans.len());
        let errors = render_chunks(jobs, plans, workers, render, &mut j, &|_| {})?;
        Ok((j.finish()?, errors))
    }

    fn run(jobs: Vec<Fake>, workers: usize) -> (Pooled, Vec<Progress>) {
        let n = jobs.len();
        let events = RefCell::new(Vec::new());
        let mut j = joiner(n);
        let r = render_chunks(jobs, &plans(n), workers, fake_render, &mut j, &|p| {
            events.borrow_mut().push(p)
        })
        .and_then(|errors| Ok((j.finish()?, errors)));
        (r, events.into_inner())
    }

    fn hap(bar: i64, m: &str) -> EngineRenderError {
        EngineRenderError::new(ErrorKind::Hap, m, Some(bar))
    }

    #[test]
    fn contents_come_back_in_chunk_order() {
        for workers in [1, 2, 4, 8] {
            // Later chunks finish first.
            let jobs: Vec<Fake> = (0..6)
                .map(|i| Fake {
                    delay_ms: (6 - i as u64) * 3,
                    result: Ok(vec![hap(i as i64, "h")]),
                    ..ok(i)
                })
                .collect();
            let (r, progress) = run(jobs, workers);
            let (joined, errors) = r.unwrap();
            // Each chunk's content frame at its bar.
            let want: Vec<Vec<f32>> = (0..2)
                .map(|c| (0..6).map(|i| i as f32 + 10.0 * c as f32).collect())
                .collect();
            assert_eq!(joined, want, "workers {workers}");
            let bars: Vec<Option<i64>> = errors.iter().map(|e| e.bar).collect();
            assert_eq!(bars, (0..6).map(Some).collect::<Vec<_>>());
            // Exactly one ChunkRendered per chunk, distinct indices.
            let mut seen: Vec<usize> = progress
                .iter()
                .map(|p| match p {
                    Progress::ChunkRendered { index, of: 6 } => *index,
                    other => panic!("{other:?}"),
                })
                .collect();
            seen.sort_unstable();
            assert_eq!(seen, (0..6).collect::<Vec<_>>(), "workers {workers}");
        }
    }

    #[test]
    fn a_job_error_fails_with_the_lowest_failing_chunk() {
        for workers in [1, 3, 8] {
            let mut jobs: Vec<Fake> = (0..8).map(ok).collect();
            jobs[5].result = Err("five".into());
            jobs[2] = Fake {
                result: Err("two".into()),
                delay_ms: 30,
                ..ok(2)
            };
            let (r, progress) = run(jobs, workers);
            let e = r.unwrap_err();
            assert_eq!(e.to_string(), "render failed: chunk 2: two", "{workers}");
            assert!(
                !progress
                    .iter()
                    .any(|p| matches!(p, Progress::ChunkRendered { index: 2, .. })),
                "{workers}"
            );
        }
    }

    #[test]
    fn a_failure_stops_new_chunks() {
        let rendered = AtomicUsize::new(0);
        let jobs: Vec<Fake> = (0..50)
            .map(|i| Fake {
                result: if i == 0 {
                    Err("zero".into())
                } else {
                    Ok(vec![])
                },
                ..ok(i)
            })
            .collect();
        let r = render_with(jobs, &plans(50), 1, |j: &Fake| {
            rendered.fetch_add(1, Ordering::SeqCst);
            fake_render(j)
        });
        assert_eq!(r.unwrap_err().to_string(), "render failed: chunk 0: zero");
        // One worker: chunk 0 fails and sets the stop flag before it takes another chunk.
        assert_eq!(rendered.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_surviving_unsupported_error_fails() {
        let mut jobs: Vec<Fake> = (0..3).map(ok).collect();
        jobs[1].result = Ok(vec![
            hap(1, "fine"),
            EngineRenderError::new(ErrorKind::Unsupported, "s=kick", Some(1)),
        ]);
        let (r, _) = run(jobs, 2);
        assert_eq!(
            r.unwrap_err().to_string(),
            "render failed: chunk 1: unsupported: s=kick"
        );
    }

    #[test]
    fn a_wrong_shape_fails() {
        let mut jobs: Vec<Fake> = (0..2).map(ok).collect();
        jobs[1].frames = 2;
        let (r, _) = run(jobs, 1);
        assert_eq!(
            r.unwrap_err().to_string(),
            "render failed: chunk 1: rendered 2 frames, the plan says 3"
        );
        let mut jobs: Vec<Fake> = (0..2).map(ok).collect();
        jobs[0].channels = 4;
        let (r, _) = run(jobs, 1);
        assert_eq!(
            r.unwrap_err().to_string(),
            "render failed: chunk 0: rendered 4 channels, a mix has 2"
        );
    }

    #[test]
    fn a_short_plan_fails_in_content_of() {
        let mut p = plans(1);
        p[0].window_start_frame = 3;
        let r = render_with(vec![ok(0)], &p, 1, fake_render);
        assert_eq!(
            r.unwrap_err().to_string(),
            "render failed: chunk 0: contentOf: channel 0 has 3 frames, needs 4 (3 + 1)"
        );
    }

    #[test]
    fn jobs_and_plans_must_pair() {
        let r = render_with(vec![ok(0)], &plans(2), 1, fake_render);
        assert_eq!(
            r.unwrap_err().to_string(),
            "render failed: 1 jobs for 2 chunk plans"
        );
    }

    #[test]
    fn warnings_are_deduped_and_capped() {
        let plan_errors = vec![
            PlanError {
                kind: "other".into(),
                message: "console.warn x".into(),
                bar: None,
            },
            PlanError {
                kind: "hap".into(),
                message: "bad".into(),
                bar: Some(0),
            },
            PlanError {
                kind: "other".into(),
                message: "console.warn x".into(),
                bar: None,
            },
        ];
        let mut render: Vec<EngineRenderError> = (0..30)
            .map(|i| EngineRenderError::new(ErrorKind::Hap, format!("m{i}"), Some(i)))
            .collect();
        render.insert(1, hap(0, "m0"));
        render.push(EngineRenderError::new(ErrorKind::Ir, "ir", None));
        render.push(EngineRenderError::new(ErrorKind::Other, "o", Some(-1)));
        let w = warnings(&plan_errors, &render);
        assert_eq!(w[0], "plan: other: console.warn x");
        assert_eq!(w[1], "plan: hap: bad (bar 1)");
        assert_eq!(w[2], "render: hap: m0 (bar 1)");
        assert_eq!(w[3], "render: hap: m1 (bar 2)");
        assert_eq!(w[21], "render: hap: m19 (bar 20)");
        // 30 distinct haps + ir + other = 32 unique lines; 20 listed, 12 more.
        assert_eq!(w[22], "render: and 12 more warnings");
        assert_eq!(w.len(), 23);
        // Under the cap: everything listed, no count line.
        let w = warnings(
            &[],
            &[
                EngineRenderError::new(ErrorKind::Ir, "ir", None),
                EngineRenderError::new(ErrorKind::Other, "o", Some(-1)),
            ],
        );
        assert_eq!(w, vec!["render: ir: ir", "render: other: o (bar 0)"]);
        // The planner's own cap.
        let many: Vec<PlanError> = (0..25)
            .map(|i| PlanError {
                kind: "other".into(),
                message: format!("p{i}"),
                bar: None,
            })
            .collect();
        let w = warnings(&many, &[]);
        assert_eq!(w.len(), 21);
        assert_eq!(w[20], "plan: and 5 more warnings");
        assert!(warnings(&[], &[]).is_empty());
    }
}
