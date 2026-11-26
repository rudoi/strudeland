# 002: split planning from rendering

## Goal

`plan::plan` stays the only V8 caller. Everything after it becomes `render_planned`. That
covers the job decode and refusal, both render passes, the joins, the isolated-body checks and
the summary. It takes a `PlannedRender` and an injected `ChunkRenderer`, so every rule can be
tested on hand-built plans with synthetic PCM. There is no behaviour change.

## Prerequisites

- **Stage 001 is committed,** and the characterization passes.
- **Read the proposal's** "Planning split (stage 002)".

## Implementation

1. **`src/lib.rs`: add the chunk renderer.**

   ```rust
   pub(crate) trait ChunkRenderer: Sync {
       fn render(&self, job: &EngineJob, stems: Option<&StemLayout>)
           -> Result<Rendered, strudeland_engine::JobError>;
   }
   pub(crate) struct EngineChunks;
   ```

   `EngineChunks` calls `strudeland_engine::render_with_stems(job, layout)` for `Some` and
   `strudeland_engine::render(job)` for `None`.
2. **`pub(crate) struct PlanTiming { pub started: Instant, pub plan_ms: f64 }`.**
3. **`render_planned`:**

   ```rust
   pub(crate) fn render_planned(planned: PlannedRender, timing: PlanTiming, opts: &RenderOptions,
       mode: AudioMode, renderer: &dyn ChunkRenderer, progress: &dyn Fn(Progress))
       -> Result<PipelineAudio, RenderError>
   ```

   Its body is `lib.rs:105-246` moved verbatim. `planned` is taken by value, and `let mut
   planned` is used for `take_jobs`. There are two substitutions:
   - **The opening pass's closure** becomes `|job| renderer.render(job, stems.as_ref())`.
   - **The repeat pass's `strudeland_engine::render`** becomes `|job| renderer.render(job,
     None)`.

   `total_ms` is `ms_since(timing.started)` and `plan_ms` is `timing.plan_ms`.
4. **`render_audio_for`:**
   - `let started = Instant::now()`;
   - `render::validate_jobs(opts.jobs)?`;
   - the preset match;
   - `let (planned, plan_ms) = plan::plan(code, opts, preset)?`;
   - `render_planned(planned, PlanTiming { started, plan_ms }, opts, mode, &EngineChunks,
     progress)`.
5. **If `lib.rs` grows past readability,** move `render_planned`, `ChunkRenderer`,
   `EngineChunks`, `PlanTiming`, `render_window` and `report_chunk` into a new
   `src/pipeline_audio.rs`, and keep `lib.rs`'s re-exports. Do not rename `PipelineAudio`,
   `LoopAudio` or `AudioMode`.

## Tests

These go in `#[cfg(test)]` in the file that holds `render_planned`.

**Helpers:**

- **`job(frames, haps)`:** the minimal engine job JSON from the proposal. Confirm it passes
  `EngineJob::from_json`; add any required field the validator names.
- **`chunk(c0, c1, from_bar, to_bar, window_start_frame, frames)`:** a `PlannedChunk` with lookback
  0 and `content_frames: Some(frames)`. Mirror what `Joiner::for_plans_with_tail` accepts; read
  `join.rs` for the invariants.
- **`planned(window, chunks, export)`:** a `PlannedRender` over `testing::structure()` (from 001)
  with cps 0.5.
- **`Ramp`:** a `ChunkRenderer` that returns `channels` channels of `job.frames` samples. Each
  sample's value is `chunk_marker + i * 1e-6`, where `chunk_marker` is read from the job's
  `seed`. Two channels without stems, or `layout.ids().len() * 2 + 2` with them. It counts its
  calls (`AtomicUsize`) and returns configured `errors`.

**Cases:**

- **`a_plan_renders_and_joins_its_chunks`:** two chunks.
  - The channels equal the concatenated ramps.
  - `music_frames` and `summary.frames`, `chunks` and `jobs` are as expected.
  - Progress is `Planned{2}` then two `ChunkRendered{.., of: 2}`.
- **`unsupported_material_is_refused_before_any_render`:** a hap on value `{"s":"bd"}`.
  - The result is `Err(Unsupported)`, and the `Ramp` was called 0 times.
  - The events are exactly `[Planned{1}]`.
- **`a_tail_extends_the_window`:** `export: Some(PlannedExport { tail_frames: 100, .. })`.
  `channels[0].len() == music + 100` and `summary.frames == music + 100`.
- **`missing_frames_fall_back_to_the_window`:** `music_frames: None` and `loop_start_frame:
  None`. `loop_audio.start_frame == join::window_frames(from, start_bar, cps)`.
- **`the_isolated_body_renders_after_the_opening`:** an export with `repeat` holding one chunk.
  - Progress is `Planned{2}` and `ChunkRendered{0,2}`, then `ChunkRendered{1,2}`. The repeat's
    index is offset.
  - `loop_audio.isolated_body` is the repeat ramp plus its tail.
  - `summary.jobs` is the max of the two worker counts.
- **`isolated_body_errors`:** three plans, each with its exact text:
  - repeat without `loop_start_bar` → "render failed: the isolated body has no loop start";
  - `loop_start_frame > music_frames` → "… loop begins past the opening";
  - `repeat.music_frames` wrong → "… isolated body length differs from the opening loop".

  In each case, the opening rendered (`Ramp` calls ≥ 1) and nothing panicked.
- **`stems_pass_the_layout_and_respect_the_budget`:**
  - `AudioMode::Stems(Orbit)` gives `stems: Some(_)`, and the `Ramp` saw `Some(layout)` for
    every chunk.
  - `music_frames: Some(1 << 29)` gives the `stems::validate_budget` error before any render
    (`Ramp` calls 0).
- **`engine_errors_become_render_warnings`:** a `Ramp` with one `EngineRenderError` and a plan
  error. `summary.warnings` is `["plan: …", "render: …"]`.

The existing `tests/pipeline.rs` and `tests/stems.rs` cover the real path unchanged.

## Validation

- **Targeted:** `cargo test -p strudeland-render render_planned` (or the module name).
- **The stage gates,** logged once. `characterization` passes, and the golden is untouched.
- **`rg -n "Host::new" crates/strudeland-render/src`** shows only `plan.rs`.

## Commit message

```
refactor(render): split planning from rendering

render_audio_for now plans through the headless host and hands the plan
to render_planned, which decodes and refuses the jobs, renders both
passes through an injected chunk renderer, joins them and builds the
summary. The refusal, isolated body, tail, chunk count, progress and
stem budget rules are tested on hand-built plans with synthetic PCM.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
