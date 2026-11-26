# 0124: one planner, with every frame count in the plan

## Goal

The render harness's gates and production renders plan their engine jobs with the same code,
and every frame count the joiner needs comes from the planner.

1. **Frames in the plan.** `windowPlan` emits `contentFrames` for constant meter too, and
   `overlapFrames` whenever a chunk has overlap. `planRender` always emits `musicFrames`,
   `loopStartFrame` (with `loopStartBar`) and `repeat.musicFrames`. The Rust mirror makes them
   required, and the joiner and the pipeline stop recomputing `round(bar * sr / cps)`.
2. **One planner.** `planJob` is deleted. The harness's single-chunk requests become a `chunk`
   field on a `planRender` window request, run through the same pass code as every production
   chunk. The host loses `plan_job*`, `PlannedJob`, `RenderRequest` and the binary's `plan`
   subcommand. `BATCH_REUSE_SCHEDULE` and its fallback go.
3. **The batch window is a renderer capability.** An optional `Renderer.renderWindow`
   replaces the `CheckDeps.batchWindow` hook that overwrote `fx.assembled`.

**Nothing audible changes, and no job text changes.** The following stay byte for byte the same:

- every engine job's text (the job schema stays 2);
- every render through `strudeland-render` and `strudeland render`, and every stem set;
- every render goldens file, threshold, CLI flag, CLI output and `SKILL.md`;
- the gate metrics of every `render:check` mode.

## Background

The evidence is in [reverify.md](0124-one-planner/reverify.md), with file and line at
`a8151213`. In short:

- **The gates check `planJob` chunks; production renders `planRender` chunks.**
  - `planJob` (`src/lib/render/job-plan.ts:201-231`) serves the render page's `job` kind
    (`src/render/main.ts:154`) and the host bundle (`src/host/main.ts:187`). Through the
    bundle it serves `Host::plan_job*` (`crates/strudeland-host/src/host.rs:104-141`) and the
    binary's `plan`.
  - Its users are all harness:
    - `render:check --engine rust` with every planner;
    - `engine:refs`, `host:parity`, `strudeland-host bench` and `perf_budget`.
  - `strudeland render` and the app export go through `strudeland_render::plan`, then
    `Host::plan_render`, then `planRender` (`job-plan.ts:535-701`).
  - Two things tie the two planners together:
    - `host:parity --batch`;
    - `BATCH_REUSE_SCHEDULE` (`:295`) with the test-only `reuseSchedule` argument
      (`:542`, `:627-629`). Its test is at `tests/render-job-plan.test.ts:887-899`.
- **Frames only for mixed meter.**
  - `windowPlan` (`src/lib/render/plan.ts:126-172`) sets `contentFrames` and `overlapFrames`
    only when it is given a timing.
  - `planRender` adds `musicFrames`, `loopStartFrame` and `repeat.musicFrames` only with a
    timing.
  - For constant meter, three places recompute them:
    - Rust, in `strudeland-render/src/join.rs:14-30` (`window_frames`, `frame_of`, the
      `content_frames` fallback, `Joiner::new`);
    - Rust, in `pipeline_audio.rs:80-87`, plus the optional repeat check at `:129`;
    - `batch-window.ts`, at `:136-141`, `:157` and `:175`.
- **The recomputation is exact, so emitting the frames changes nothing.**
  - `windowFrames` (`contract.ts:115-121`) is `Math.round(bar*sr/cps)` minus the same for the
    other edge.
  - `frame_of` (`join.rs:19-21`) does the same operations.
  - `cps` survives serde exactly (`float_roundtrip`).
  - So the four constant-meter fields equal what Rust computes today, with no ±1 risk:
    `contentFrames`, `musicFrames`, `loopStartFrame` and `repeat.musicFrames`.
- **`--planner batch` is a hook.**
  - `CheckDeps.batchWindow` (`scripts/render/check.ts:144-146`) overwrites `fx.assembled`
    (`:618-621`) before the gates read it.
  - Nothing in the `Renderer` contract says a renderer can produce a whole window.

## Scope

In:

1. **Stage 001, frames.**
   - TS: `windowPlan` emits `contentFrames` in both branches and `overlapFrames` with overlap,
     using one `renderWindowFrames` expression. `planRender` always emits `musicFrames`,
     `loopStartFrame` (with `loopStartBar`) and `repeat.musicFrames`. `WindowPlan.contentFrames`
     and `RenderResult.contentFrames` become required, and every presence check is deleted.
   - Rust: the fields become required, and the recompute fallbacks are deleted.
     `window_frames` moves to `strudeland_render::testing`.
   - `batch-window.ts` reads frames and seams from the `--info` chunks.
   - The `joiner.json` generator emits per-chunk frames.
   - The A/B baseline, recorded before any edit.
   - The stale comment at `schedule.ts:166` (task #151, a planner file this stage regenerates
     anyway).
2. **Stage 002, one planner.**
   - `RenderPlanRequest.chunk` (TS and Rust) and the single-chunk path in `planRender`.
   - `planJob`, `BATCH_REUSE_SCHEDULE`, `reuseSchedule`, the bundle's `planJob` and the
     `__strudelandJob` slot are deleted. So are, in Rust, `Host::plan_job*`, `Work::Plan`,
     `PlannedJob`, `RenderRequest` and the binary's `plan`.
   - The page's `job` kind, `HostPlanner`, `bench`, `perf_budget` and `host:parity` move onto
     the single-chunk request.
   - Task #66 item 1: `isObject` becomes `isRecord`, in the validator this stage edits.
3. **Stage 003, batch adapter.**
   - `Renderer.renderWindow?`.
   - `PipelineRenderer`, which wraps the host-planned `RustRenderer` and the
     `strudeland-render` process.
   - `assembledOf` uses `renderWindow` when the renderer has it.
   - `CheckDeps.batchWindow`, `createBatchWindow` and the `fx.assembled` overwrite are deleted.

Out:

- **Full `RenderTiming` unification**, meaning constant meter as a degenerate timing and one
  `windowPlan` branch.
  - The legacy `round((from-c0)*sr/cps)` differs from the grid
    `round(from*sr/cps) - round(c0*sr/cps)` in 39,539 of 192,960 swept cases (20%), always by
    ±1 (`reverify.md`).
  - So moving `windowStartFrame` or `frames` onto the grid changes `job.frames` and the cut
    offset, which is audio.
  - A degenerate timing would also move:
    - `hapTiming` (`t`, plus an added `phaseSec`);
    - `historyStart`;
    - the pause schedule;
    - superdough's `offline.ts`.
  - The legacy arithmetic for `c0`, `windowStartFrame`, `frames`, `bufferStartSec` and hap
    timing stays exactly as it is. Changing it needs the user's decision on an audio change.
- **Keeping `plan_job` for the harness.** It is what the gates plan with and production does
  not. Keeping it keeps two planners.
- **Batch as a `check.ts` hook.** That is the thing stage 003 removes.
- **A mutation knob in the planner.** Mutations stay applied to the planned job by
  `RustRenderer` (`applyMutation`), as today. The planner never sees them.
- **Unifying the Rust and TS request validation** (task #66 item 2), and the other #66 items.
- **The `assembleWindow` port in TS.** It stays: the superdough harness joins chunks with it.
- **Any change to `strudeland-engine`.** The engine never reads the frame fields.

## Technical design

### Stage 001: frames in the plan

#### TypeScript

- **`windowPlan`** (`plan.ts`). After the `c0`/`c1` computation, and outside the branch:
  - `contentFrames = renderWindowFrames(req.fromBar, req.toBar, cps, req.sampleRate, timing)`;
  - when `req.overlapBars > 0`, `overlapFrames = renderWindowFrames(req.toBar, c1, …, timing)`.

  The timing branch gives the same numbers it gives today, because `to - from` is
  `renderWindowFrames` with a timing. The constant branch's `frames`, `windowStartFrame`,
  `lookbackSecUsed` and `bufferStartSec` keep their legacy arithmetic. Only the two measures
  are added.
- **`WindowPlan`.**
  - `contentFrames: number` becomes required.
  - `overlapFrames?: number` stays optional. It is present exactly when `overlapBars > 0`.
  - The doc comments say so.
- **`PlannedRender`.**
  - `musicFrames: number` becomes required.
  - `export.loopStartFrame?: number` is present exactly when `export.loopStartBar` is.
  - `repeat.musicFrames: number` becomes required.
  - `RenderPass.musicFrames` follows.

  The three `...(timing ? … : {})` spreads at `job-plan.ts:672`, `:682-684` and `:691` become
  plain fields. `framesBetween` already computes the values: `musicalFrames` and `bodyFrames`
  are computed today and simply not emitted.
- **`RenderResult.contentFrames: number`** (`contract.ts`) becomes required. `overlapFrames`
  stays optional, as on `WindowPlan`. The presence checks change as follows:

  | Where | Today | After |
  |---|---|---|
  | `chunks.ts:217-219` `contentFrames(r)` | `r.contentFrames ?? windowFrames(…)` | deleted; callers read `r.contentFrames` |
  | `chunks.ts:229` `overlapOf` | `r.overlapFrames ?? windowFrames(toBar, toBar + overlapBars)` | `0` when `overlapBars` is 0; `r.overlapFrames` otherwise; a `RangeError` when it is absent |
  | `chunks.ts:290-294` `assembleWindow` | branch on `first.contentFrames === undefined` | always `bufferStartSec: first.bufferStartSec + first.lookbackSecUsed` and `contentFrames: frames` |
  | `features.ts:137` | `result.contentFrames ?? windowFrames(…)` | `result.contentFrames` |
  | `src/render/main.ts:208` | conditional spread | `contentFrames: plan.contentFrames`; `overlapFrames` stays conditional |
  | `scripts/render/rust-engine.ts:514` | conditional spread | `contentFrames: info.plan.contentFrames`; `overlapFrames` stays conditional |
  | `batch-window.ts:175` | `timing ? { contentFrames } : {}` | `contentFrames: frames` |

- **One harness side effect** (`reverify.md`, "One harness side effect"). `assembleWindow` now
  takes its mapped `bufferStartSec` for constant meter too:
  `first.bufferStartSec + first.lookbackSecUsed`, that is `c0/cps + (from-c0)/cps`.
  - On the corpus, only `late-feedback` (from 136) and `crowded-bass` (from 4) start past bar
    0, and for both that sum equals `from/cps` exactly. For a window starting at bar 0 both
    terms are 0.
  - The A/B report diff (below) is the absolute check that no gate moved.
  - If it shows a difference, **stop**; do not special-case the branch back.
- **`batch-window.ts`.**
  - `parseBatchInfo` requires each chunk's `contentFrames`, a non-negative integer.
  - `batchAssembled`'s frames are the sum of the chunks' `contentFrames`.
  - `batchSeamSecs` is the running sum before each later chunk, divided by `SAMPLE_RATE`.
  - These equal today's `renderWindowFrames` results because the edges telescope.
  - `bufferStartSec` keeps its formula, so `timing` stays a parameter for that alone.
- **`scripts/render/loudness-vectors.ts`.**
  - `chunkResult` sets `contentFrames: windowFrames(from, to)`. It also sets `overlapFrames`
    when the chunk has overlap.
  - `joinerCases` writes each chunk's `contentFrames` and buffer `frames` into `joiner.json`.
    That is `render:vectors --write`.
  - The vector file's `schema` stays 1. The shared vector writer stamps 1 on every file, and
    the Rust reader requires the new keys, so a stale file fails to parse.
- **The comment at `src/lib/render/schedule.ts:166`** names `coerce.rs` (task #151).

#### Rust

- **`crates/strudeland-host/src/planned.rs`.**
  - `PlannedRender.music_frames: u64`, `PlannedRepeat.music_frames: u64` and
    `ChunkWindow.content_frames: u64` become required: no `default`, always serialized.
  - `ChunkWindow` gains `overlap_frames: Option<u64>` (skip when `None`), so a chunk's overlap
    measure survives the host's re-serialization. Stage 002 sends overlap through
    `plan-render`.
  - `PlannedExport` keeps `loop_start_bar: Option<u32>` and `loop_start_frame: Option<u64>`.
    The wire shape stays the flat pair the TS writes, and the doc says they are present
    together.
- **`pipeline_audio.rs`.**
  - `music_frames` comes from the plan.
  - The loop start is `match (bar, frame)`: both present is `Some((bar, frame))`, both absent
    is `None`, and a half is a `RenderError::Render("the plan's loop start has no frame")`
    (or "no bar").
  - The repeat check `repeat.music_frames != expected` is unconditional.
  - The two `join::window_frames` fallbacks are deleted.
- **`join.rs`.**
  - `content_frames()` is deleted; `kept_range` reads `plan.content_frames`.
  - `Joiner` loses its `cps` field and parameter. `with_tail` becomes the public
    `Joiner::new(window, spans: &[(Span, u64)], music_frames: u64, tail_frames: u64)`.
    `for_plans(window, plans, channels, music_frames, tail_frames)` replaces `for_plans` and
    `for_plans_with_tail`.
  - The music-frames check is always made.
  - `window_frames` and `frame_of` move to `crate::testing`, which is already compiled into
    non-test builds and never unwraps. `characterization.rs`, `tests/pipeline.rs` and
    `tests/joiner_vectors.rs` take them from there. `join.rs`'s
    `window_frames_matches_the_ts_rounding` test moves with them.
- **`tests/joiner_vectors.rs`** reads each chunk's `contentFrames` and `frames` from the vector
  file. It no longer computes buffer frames. It still checks
  `testing::window_frames(window) == case.frames`, which pins the helper the other tests use
  against the TS.

#### What the A/B guard sees

- Audio, job text and the gate metrics: no change.
- `--info` files: each chunk gains `contentFrames` for constant meter. Mixed-meter chunks
  already have it, with the same value.
- `host:parity --mixed-meter` compares whole plans and must still be equal. The mixed-meter
  plans already carried every field.

### Stage 002: one planner

#### The request

```ts
/** A harness request for one chunk: exactly the job `[fromBar, toBar)` plus overlap, lookback
 *  and mode would get, planned by the pass code every production chunk goes through. */
export interface SingleChunk {
  overlapBars: number;
  lookbackSec: number | 'auto';
  mode: RenderMode;
}

export interface RenderPlanRequest {
  preset?: ExportPreset;
  length: LengthRequest;
  includeEvents?: boolean;
  /** Plan exactly one chunk over `length` (a window), for the harness. */
  chunk?: SingleChunk;
}
```

- **Where `chunk` lives.** It sits beside `length`, not inside the window kind. The Rust mirror
  is `RenderPlanRequest.chunk: Option<SingleChunk>`, and the two must agree on the wire.
  - The production entry point, `strudeland_render::plan`, builds its request from
    `RenderOptions`, which has no chunk. So `strudeland render` and the app cannot send one,
    and no production refusal is needed.
  - The bundle checks the combination.
- **`validateRenderRequest` refuses each of these six** with `bad request: …`:
  - a `chunk` that is not an object;
  - `chunk` without `length.kind === 'window'`;
  - `chunk` with a `preset`;
  - `overlapBars` that is not an integer with `toBar + overlapBars <= MAX_RENDER_BARS`;
  - a `lookbackSec` that is not `'auto'` or a finite number `>= 0`;
  - a `mode` other than `mix` or `stems`.

  These are today's Rust `RenderRequest::validate` rules, which the bundle now owns. `isObject`
  becomes `isRecord` from `engine-job.ts` (task #66 item 1).
- **Rust `SingleChunk`** is `{ overlap_bars: u32, lookback_sec: LookbackSec, mode: Mode }`,
  `camelCase` and `deny_unknown_fields`. `LookbackSec` and `Mode` stay. `RenderRequest` and its
  `validate` and `to_json` are deleted.
- **`singleChunkRequest(req: RenderRequest): RenderPlanRequest`** (`job-plan.ts`) builds the
  request from a harness `RenderRequest`:

  ```ts
  { length: { kind: 'window', fromBar, toBar, layout }, chunk: { overlapBars, lookbackSec, mode } }
  ```

  The page's `job` kind, `HostPlanner.job` and the tests use it.
- **`pageJobInfo(planned: PlannedRender): PageJobInfo`** (`page-job.ts`) does the reverse.
  - It takes the one chunk's `json.length`, `info.events`, `plan` and `info.stemPairs`, and the
    plan's `errors`, `cps` and `structure`.
  - It throws unless there is exactly one chunk with an `info`.
  - `PageJobInfo` stays the `JobSource` output, so `RustRenderer` does not change.

#### The pass code

`planRender` with `chunk` makes these changes, none of which touches a production request:

| Step | Production (unchanged) | Single chunk |
|---|---|---|
| schedule window | `selectHaps(query(scheduleStart, toBar))` | `selectHaps(query(0, toBar + overlapBars))`; `scheduleStart` is 0 (no preset) |
| `base.mode` | `'mix'` | `chunk.mode` |
| chunk requests | `renderChunkPlans(...).map(p => chunkRequest(base, p))` | one: `{ ...base, overlapBars, lookbackSec: chunk.lookbackSec === 'auto' ? history : chunk.lookbackSec }` |
| each chunk's schedule | `cutSchedule(full, req.toBar + req.overlapBars)` (`overlapBars` is 0) | the same expression; the cut keeps every hap, because they all start before `c1` |
| chunk `info` | absent | `{ events: planned.events, stemPairs: planned.stemPairs }` |
| `lookbackSec`, `musicFrames`, `errors`, `events` | as today | the same rules: `lookbackSeconds` of `[0, c1)`, `framesBetween(fromBar, toBar)`, the deduped chunk errors, the schedule events with `includeEvents` |

- **`BATCH_REUSE_SCHEDULE` and `reuseSchedule` are deleted.** Each chunk's schedule is always
  the cut. `planWindow` keeps `info` local.
  - It returns `{ info, json }` as today.
  - The pass copies `info.events` and `info.stemPairs` into the chunk only when `chunk` is
    set.
  - This needs `planWindow`'s `pageJob` result to carry `stemPairs`. It already does.
- **Byte-equality with `planJob`.** `planJob` selected `[0, c1)` once, resolved `'auto'` from
  it, then called `planWindow(ctx, request, lookback, schedule, captured)` with
  `scheduleStart` 0 and no tail. The single-chunk path calls the same `planWindow` with the
  same arguments:
  - the cut of `[0, c1)` at `c1` is the same selection, as a new array of the same objects;
  - `frozenSchedule` is false;
  - `ctx.structure` is the evaluated structure.

  So the job text and `info` are the same.
- **Absolute checks** (`planJob` no longer exists to compare against):
  - `engine:refs` plans every micro case and fixture job through the page's `job` kind and must
    show provenance-only diffs.
  - `ab.sh reports-check` compares the sorted sha256 of every job text that
    `render:check --engine rust` kept, for the page and the host planners, against the
    baseline.

#### Callers

- **Render page** (`src/render/main.ts`). The `job` kind runs
  `planRender(rt, request.code, singleChunkRequest(request), captured, onPhase)`. Then:
  - `jobJson = planned.chunks[0].json`;
  - `jobInfo = pageJobInfo(planned)`.

  `StartArgs` and the driver (`superdough-renderer.ts`) do not change.
- **Host bundle** (`src/host/main.ts`). Delete `planJob`, `runPlanJob`, `StrudelandHost.planJob`
  and the `__strudelandJob` slot. `planRender` takes `chunk` through `RenderPlanInput`
  unchanged.
- **Rust host.**
  - Delete `Host::plan_job`, `plan_job_with_timeout`, `plan_job_timed`, `Work::Plan`,
    `PlannedJob` and the `lib.rs` re-exports.
  - `Work::PlanRender` gains `profile: bool`, and a crate-private `plan_render_timed` returns
    `(PlannedRender, PlanTimings)`. `PlanTimings.json` becomes the read-out plus the plan's
    parse.
  - `PlannedChunk` gains `info: Option<ChunkInfo>`, with
    `ChunkInfo { events: serde_json::Value, stem_pairs: u32 }` (`camelCase`, skipped when
    `None`).
  - Events pass through `serde_json::Value`, as `PlannedRender.events` already does:
    - the numbers round-trip exactly (`float_roundtrip`);
    - object keys may come back sorted;
    - every reader (`applyMutation`, the gates, `firstDifference`) reads by key.
- **The `strudeland-host` binary.**
  - `plan` is deleted. The usage, the module doc and `README` lose it.
  - The planning timeout stays the request's `timeoutSec` on `plan-render`, the field
    `strudeland render` already uses. No new flag.
  - `strudeland-host plan …` now exits 2 with `unknown command plan`.
- **`bench` and `perf_budget`.**
  - `bench::request(bars)` returns the single-chunk window request: linear `[0, bars)`, overlap
    0, lookback 0, mix.
  - `one_run` calls `plan_render_timed` and takes `chunks[0].json` as the job. So the job's
    size and hash in the report are unchanged.
  - `perf_budget` plans the same fixture through `plan_render`. The 2 s budget then measures
    the same planning plus the plan's JSON round trip.
  - **Never loosen `BUDGET`.** If the median goes over, stop and report.
- **`HostPlanner`** (`scripts/render/host-planner.ts`).
  - `job(req)` runs `plan-render` with `JSON.stringify(singleChunkRequest(req))` (no `code`;
    the song is the file) and returns `{ job, text: chunks[0].json, info: pageJobInfo(plan) }`.
  - `job` stays lazily parsed.
  - `hostRequestJson`, `hostPlanArgs` and `parsePlanOutput` are deleted. `parsePlanRenderOutput`
    keeps its checks, and `pageJobInfo` refuses a plan without exactly one chunk with `info`.
  - The `info` now passes through the Rust mirror, where today's `plan` printed the bundle's
    text: `ChunkWindow` (no unknown key kept), `Structure`, `PlanError` (`bar: Option<i64>`)
    and events as `serde_json::Value`. A seam chunk's `overlapFrames` survives only because
    001 adds `ChunkWindow.overlap_frames`, which `overlapOf` needs. `host:parity` checks that
    nothing is lost (below).
- **`host:parity`.**
  - Non-batch: each request in the set goes through the page's `job` kind and through
    `HostPlanner.job`. Both are single-chunk `planRender` now. The job text is compared byte
    for byte, as today, and the `info` deep-equal without `errors`, which is new. The seam
    requests of the seven `seam`-gated fixtures cover `plan.overlapFrames`.
  - `--batch` does two things:
    1. it compares the page's `plannedRender` with the host's `plan-render` of the fixture
       window as whole plans, through `firstDifference`, with `errors` removed from both;
    2. it keeps today's per-chunk check: each host chunk's `json` against the page's
       single-chunk job at that chunk's bars and lookback, with overlap 0.

    `errors` is left out because the page and the host capture errors differently (host
    README, "Known differences"). Part 2 is the corpus-scale evidence that the cut schedule
    equals a chunk's own query, the property `BATCH_REUSE_SCHEDULE`'s fallback existed to
    guard.
  - `--mixed-meter` keeps its whole-plan comparisons, and adds two single-chunk requests over
    the mixed-meter song (one with overlap across the 3/8 bar), compared by job text and by
    `info` without `errors`. No corpus fixture has `meterChanges`.

#### Tests (replace, don't layer)

| Today | After |
|---|---|
| `render-job-plan.test.ts` "every chunk's job equals planJob of its chunk request" (`:838-885`) | "a single-chunk request at chunk k's bars and lookback, overlap 0, equals planRender chunk k": the same five cases, comparing `json` and `plan` (the single chunk's `plan` equals chunk k's) |
| "the per-chunk schedule fallback gives the same jobs" (`:887-899`) | deleted |
| `describe('planJob success'…)`, `planJob query reuse`, `planJob failures` | the same tests on `planRender(rt, code, singleChunkRequest(req), …)`, reading `pageJobInfo`; the `expected(...)` helper still builds the want from `planEngineJob` directly, which makes them absolute. In `planJob failures` (`:424-447`): the three cps cases become `windowPlan` tests (extending `render-offline.test.ts:282`); the lookback and empty-window rows become `validateRenderRequest` refusals with their `bad request: …` messages; the buffer bound is rewritten at `toBar: 100000` |
| — | `validateRenderRequest`: the six `chunk` refusals, the moved rows, and an accepted chunk |
| `host-entry.test.ts` `planJob: …` (`:85-187`) | the same behaviours through `strudelandHost.planRender` with a `chunk`; tests that duplicate an existing `planRender:` test are deleted, not ported; the window-less request (`:181`) becomes a bad `chunk`, kind `plan` with `bad request: …` |
| `render-host-planner.test.ts` `hostRequestJson`, `hostPlanArgs`, `parsePlanOutput` | `singleChunkRequest` JSON, the `plan-render` argument list, and `parsePlanRenderOutput` to `PageJobInfo` (with the same exit, signal, truncation and malformed cases) |
| `tests/render-page-job.test.ts` | adds `pageJobInfo`: one chunk, and the throw on zero, two, or no `info` |
| `crates/strudeland-host/src/request.rs`: `RenderRequest` validation tests | `SingleChunk` serde: its shape, `auto` and a number, unknown fields refused; the range checks are the TS tests above |
| `tests/host.rs` `plan_job` tests | onto `plan_render` with a chunk; those that duplicate `tests/plan_render.rs` are deleted (`a_request_does_not_see_the_previous_ones_globals`, `a_song_waiting_on_a_timer_is_unsettled`, `an_invalid_request_is_plan`, merged into the `plan_render.rs` one) |
| `tests/plan_render.rs` `chunks_equal_plan_job_of_each_chunk_request` | `chunks_equal_the_single_chunk_plan_of_each_chunk_request` |
| `tests/bin.rs` `plan_*` tests | ported to `plan-render` where it lacks the behaviour (a request file, `--out`, exit 1 with the kind, exit 2 with the usage, `timeoutSec`); plus `plan` is an unknown command |
| `tests/never_panics.rs` odd requests | the same loop plus the odd `chunk` values a typed `SingleChunk` can hold (`overlap_bars: u32::MAX`; a NaN, infinite or negative `lookbackSec`); every other `plan_job` call there (`:69`, `:87`, `:195`, `:225`, `:234`, `:253`, `:261`) ported to `plan_render` with a chunk |
| — | `tests/bin.rs`: the odd `chunk` values a typed request cannot hold, as raw `--request` JSON (a negative or fractional `overlapBars`, a non-number `lookbackSec`, an unknown `mode` or key): a serde `invalid request`, exit 2 |
| `tests/bench.rs` "the hash and size are those of the plan output" | against `plan-render`'s `chunks[0].json` |
| `tests/common/mod.rs` `request(to_bar)` | returns the single-chunk `RenderPlanRequest` |
| `crates/strudeland-render/tests/joiner_identity.rs` | each per-chunk plan is `Host::plan_render` with a chunk (a fresh host each), compared with the batch's job text, then rendered and joined; the same bit-identity assertion |

### Stage 003: the batch window as a renderer capability

```ts
export interface Renderer {
  // …
  /** Renders `[base.fromBar, base.toBar)` of `base.code` as one window through the renderer's
   *  own production path. When present, the harness takes the fixture's main window from it
   *  instead of joining chunk renders. `events` is the harness schedule, attached as
   *  `assembleWindow` attaches it. */
  renderWindow?(base: RenderRequest, events: RenderEvent[]): Promise<AssembledWindow>;
}
```

- **`AssembledWindow`** stays in `chunks.ts`, and `contract.ts` imports its type. That is a
  type-only cycle, and `contract.ts` stays free of runtime imports.
- **`PipelineRenderer`** (`scripts/render/batch-render.ts`) implements `Renderer`. It is built
  from the host-planned `RustRenderer` and a `BatchRenderer`.
  - `schedule`, `render`, `version` and `dispose` delegate to the `RustRenderer`.
  - `engine` is `'rust'`.
  - `renderWindow(base, events)` runs these steps:
    1. it reads `inner.schedule(base)`, which is cached (`prepareFixture` already fetched it);
    2. it derives `timing = renderTiming(structure, layoutSegments(base.layout, structure),
       cps)`, the same derivation `check.ts:518` uses for `fx.timing`;
    3. it writes the song to `<outDir>/batch/<sha256>.js` with `writeSongFile`;
    4. it runs `BatchRenderer.window` with `name: <sha8>-<from>-<to>-<layout>`. The layout is
       in the name because `song-linear` and `song-clip` are one song in two layouts.
  - The cross-checks (window, cps, schedule, errors, tiling) are unchanged.
  - `BatchTarget` loses nothing; `PipelineRenderer` fills it.
- **`check.ts`.**
  - `assembledOf(fx, tx, batch)`: with no `tx` and a renderer that has `renderWindow`, the
    window is `fx.limit.run(() => fx.renderer.renderWindow!(fx.base, fx.sched.events))`. That
    happens once, memoized in `fx.assembled`, as today.
  - Otherwise `assembledOf` joins the chunk renders, as today.
  - Mutations (`tx`) always join chunk renders. Twin, seam, stems and the mutations keep the
    per-request host plans, as the module doc says today.
  - Deleted: `CheckDeps.batchWindow`, `createBatchWindow` and the `ctx.assembled = a`
    overwrite at `:618-621`. That call site becomes `assembledOf(fx, undefined, batch)`.
  - `createRenderer` returns the `PipelineRenderer` for `--planner batch`.
- **Tests.**
  - The `render-check-flow.test.ts` `runCheck --planner batch` cases (`:1059`, `batchOf`) become fake renderers
    with `renderWindow`. They check:
    - the main window comes from it;
    - mutations still join chunks;
    - a failure is the fixture's error.
  - `render-batch-window.test.ts` keeps its pure tests.
  - A `PipelineRenderer` test with a fake inner renderer and runner checks two things: the
    song file and arguments, and that the timing it derives equals the fixture's.

## The A/B guard

No committed hash covers a corpus song through the whole pipeline, so stage 001 records an
absolute baseline **before any edit**. The draft script is
`$S/0124-draft/ab.sh`, where `$S` is the planning session's scratchpad. Stage 001 copies it to
`$S/0124-baseline/ab.sh`. It refuses `baseline` and `reports-baseline` unless nothing is
edited or added, against `a8151213`, under `src`, `crates`, `scripts`, `tests`, `vendor`,
`Cargo.toml`, `Cargo.lock`, `package.json`, `bun.lock`, `strudel-pins.json`,
`vite.render.config.ts` and `render/index.html`.

- **`ab.sh baseline` / `check`** (96 cases). Release `strudeland-render` and `strudeland` built
  from the tree render:
  - every corpus fixture's window (34), with `--loudness off --format wav --bit-depth 32f
    --info`;
  - `song-linear.js`, `tests/render/mixed-meter-loop.js` and a scratch song, `frac-737`:
    - clip, `--loops 1` and `3`, every section, `--length 3` and `11`, through
      `strudeland-render`;
    - every preset, raw and leveled, a one-shot section, a leveled plain clip and an orbit stem
      set, through `strudeland render`;
  - the stems fixtures' orbit stem sets (`stems-5`, `stems-shared`, `late-feedback`).

  `frac-737` is written by the script into `$S/0124-baseline/songs/`, never under `tests/`.
  It is 137 bpm in 7/8: a bar is 67598.54 frames, so every bar edge after 0 rounds, and it
  has an intro, a loop section and a tail. The corpus has fractional bars only as plain
  windows (`odd-meter` is also 137 bpm 7/8). The two sectioned songs that go through the
  presets, `--loops` and `--section` have whole-frame bars at 120 bpm, so without `frac-737`
  `loopStartFrame` and `repeat.musicFrames` would be checked only where rounding does
  nothing.

  It records every output file's sha256, every exit code, the stderr hash of each failed case
  (`samples` and `noise-seed` are refused), the stdout of every `strudeland render` case
  without its wall-clock `time:` line, and the `--info` JSON with its timings removed.
  `baseline` records twice and requires the two to be equal (determinism). It copies the
  script and `base/` to `~/.cache/strudeland-0124/`. `check` compares audio exactly. An
  `--info` file may differ in one way only: a chunk that lacked `contentFrames` gains it.
- **`ab.sh reports-baseline` / `reports-check`.** These run four `render:check` modes:
  - superdough;
  - `--engine rust`;
  - `--engine rust --planner host`;
  - `--engine rust --planner batch`.

  The three Rust modes run with `--keep-wav`. Every run writes into one fixed folder, then is
  normalized and moved, so no output path differs between runs. The script compares:
  - each `report.json`, without `startedAt`, `engineVersion`, `chromium`, `playwright` and
    each fixture's wall-clock `seconds` (`check.ts:582`);
  - each exit code;
  - for the Rust modes, the sorted sha256 list of every kept engine job text and WAV.

  `reports-baseline` runs every mode twice and requires the two to be equal. A Rust mode that
  differs is a stop. If superdough differs only in measurements, the varying fields are
  recorded and `sd` is compared from then on by its exit code and its fixture, gate, parity
  and mutation verdicts. That is a property of this A/B guard, not a repo threshold. So the
  gate metrics (the Rust modes exactly), every planned job's text (page and host) and the
  harness's joined audio are checked absolutely.
- **Production job text** (`strudeland_render::plan`'s chunks) is compared as text by no gate.
  It is guarded through the audio hashes of `check`, and from 002 by `host:parity --batch`.
- **`ab.sh refs`.** After `engine:refs --write`, checked against `HEAD`:
  - every changed JSON file under `tests/refs` and `plans/*/evidence` (the pathspec
    `':(glob)plans/*/evidence/**'`) differs only in `provenance.plannerSha256`;
  - `index.json` also in `files[*].sha256`, which moves with each job file's provenance, and
    each listed hash must be its file's sha256 on disk, with every `bytes` unchanged (the
    precedent is `plans/0116-render-request-cleanup/proposal.md:139`);
  - the measured-floor cases (`FLOOR_CASES`, `scripts/engine/refs-lib.ts:719`:
    `vowel-formants` and `mod-lfo-note`) may change their WAV and `floorDb`. Their job bytes
    change, so `settleFloorCase` (`:777`) re-renders them, and the render varies run to run.
    No other WAV changes, and no file is added or deleted;
  - `bun run engine:refs` (check mode) then exits 0, which also enforces `FLOOR_MAX_DB`.
- **`ab.sh vectors`.** After `render:vectors --write`, `joiner.json` differs from `HEAD` only
  by the added per-chunk `contentFrames` and `frames`.

Each stage must end with `check`, `reports-check` and, where it applies, `refs` and `vectors`
equal. A difference is a **stop**:

- never re-record a baseline;
- never edit a golden, threshold, ref, vector or the A/B script to absorb a difference.

## Dependencies

- Task #116 (render request cleanup) is merged.
- Tasks #129 and #130 are blocked on this one; neither changes the design here.
- Toolchain:
  - Chromium for Playwright (`bunx playwright install chromium`), for `engine:refs`,
    `host:parity` and `render:check`;
  - cmake and a C compiler for `strudeland-render`;
  - the V8 download for `strudeland-host`.

## Risks

- **`bufferStartSec` on the mapped branch** (stage 001). This is proven equal for the corpus
  (`reverify.md`) and checked by `reports-check`. A difference is a stop, not a special case.
- **The single-chunk path drifts from `planJob`** (stage 002). The table tests rebuild the
  expected job from `planEngineJob`, `engine:refs` is absolute, and the kept job texts are
  compared absolutely. Any job-text difference is a stop.
- **`perf_budget` gets slower** (stage 002). It now includes the plan's JSON escape and serde
  parse. Measure it; never loosen `BUDGET`. If it fails, stop and report the numbers.
- **Event key order through serde** (stage 002). Keys may be re-sorted, but nothing reads
  events positionally. The kept-job hashes in `reports-check` would catch a mutation that
  depends on it (`applyMutation` serializes the mutated job).
- **The determinism of the baseline.** `baseline` and `reports-baseline` each record twice.
  If a case or a Rust report is not reproducible, the script refuses, and it is investigated
  before stage 001 starts. A superdough report that varies only in measurements is compared on
  its verdicts from then on, as the A/B guard says.
- **Run time.** The four `render:check` modes run twice before stage 001 and once after each
  stage. They
  are the slowest gates; the stage files give estimates, and the logs record the real times.

## Open questions

None. The user's decisions are recorded in the brief:

- zero audio and job-text change;
- frames only, no timing unification;
- `chunk` on the window request;
- batch as a renderer capability.

The two refinements here are noted in the stage files:

- `chunk` sits beside `length` on the wire, so the Rust and TS shapes are one;
- `--batch` keeps its per-chunk comparison alongside the whole-plan one.

## Complexity

Medium-large: about 45 files across TS, Rust and docs, in three sequential stages. Stage 002 is
the largest. The A/B guard makes each stage's equality absolute.

## Decisions

1. **Frames only.** The frame *measures* are emitted. The legacy `c0`, `windowStartFrame`,
   `frames`, `bufferStartSec` and hap timing stay. Full `RenderTiming` unification is rejected
   because it changes audio.
2. **Required fields, no recompute.** Every reader takes the planner's number, and Rust keeps
   `window_frames` only as a test helper.
3. **`overlapFrames` stays optional**, present exactly with overlap. That keeps the batch
   plans (overlap 0) identical on the wire between page and host.
4. **The loop start pair stays flat on the wire**, checked as a pair in `pipeline_audio`.
5. **`chunk` is a `RenderPlanRequest` field** in TS and Rust, valid only with a window and no
   preset. Production cannot send it.
6. **`PageJobInfo` stays the `JobSource` output.** `pageJobInfo` derives it from a single-chunk
   plan, so `RustRenderer` does not change.
7. **No `plan --timeout`.** The timeout is `timeoutSec` on `plan-render`.
8. **`--batch` compares whole plans without `errors`, and keeps the per-chunk job check.**
9. **`renderWindow` is optional on `Renderer`.** `PipelineRenderer` derives cps and timing from
   the wrapped renderer's cached schedule.
10. **No GLOSSARY term.** "Single chunk" and "planner" are harness internals; the glossary
    covers the song, export and app domains.
11. **Ride-alongs:**
    - the `schedule.ts:166` comment (task #151) in 001;
    - `isObject` to `isRecord` (task #66 item 1) in 002.

    Both are in planner files the stage regenerates anyway.
