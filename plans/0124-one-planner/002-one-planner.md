# 002: one planner

## Goal

- `planJob` is deleted. Every engine job, harness or production, is planned by `planRender`'s
  pass code.
- The harness's one-chunk requests become `RenderPlanRequest.chunk`. The page's `job` kind,
  the host bundle, `HostPlanner`, `strudeland-host bench`, `perf_budget`, `host:parity` and the
  tests all use it.
- Deleted:
  - `BATCH_REUSE_SCHEDULE`, `reuseSchedule` and the fallback test;
  - the bundle's `planJob` and `__strudelandJob`;
  - `Host::plan_job`, `plan_job_with_timeout`, `plan_job_timed`, `Work::Plan`, `PlannedJob`
    and `RenderRequest`;
  - the binary's `plan` subcommand.
- **Every job text stays byte-equal to today's `planJob` output.** Audio, goldens, thresholds,
  CLI flags and output, and `SKILL.md` do not change.

## Prerequisites

- Stage 001 is committed and its full gate is green.
- `$S/0124-baseline/` holds the baseline (`base/`, `reports/base/`).
- Read [the proposal](../0124-one-planner.md) "Stage 002" in full: the request, the pass code
  table, the callers and the test table.

## Step 1: the request (TS)

In `src/lib/render/job-plan.ts`:

1. Add `SingleChunk` and `RenderPlanRequest.chunk?: SingleChunk`, with the proposal's docs.
   `RenderPlanInput` (`contract.ts`) extends `RenderPlanRequest`, so it gains the field with
   no edit.
2. In `validateRenderRequest`:
   - replace the local `isObject` with `isRecord` from `./engine-job` (task #66 item 1);
   - validate `chunk` when it is not `undefined`, in two places. The first three rows go right
     after `length` is checked to be an object, before the existing loop-preset `meta` check,
     so that a `chunk` with a preset is a bad request and never a `meta` error. The last three
     rows go after the `length` switch, once `toBar` is known to be valid. The messages are
     each prefixed `bad request: `, through `badRequest`:

     | Condition | Message |
     |---|---|
     | `chunk` not an object | `chunk must be an object` |
     | `length.kind !== 'window'` | `chunk needs a window length` |
     | `request.preset !== undefined` | `chunk cannot be combined with a preset` |
     | `overlapBars` not an integer in `[0, MAX_RENDER_BARS - toBar]` | `chunk.overlapBars must be an integer from 0 to ${MAX_RENDER_BARS - toBar}` |
     | `lookbackSec` neither `'auto'` nor a finite number `>= 0` | `chunk.lookbackSec must be 'auto' or a number of seconds >= 0` |
     | `mode` not `'mix'` or `'stems'` | `unknown chunk mode ${String(mode)}` |

     The switch `return`s from each case today, so restructure it to `break` (or move the
     window validation into a helper) so that the chunk checks run after it.
3. Add `singleChunkRequest(req: RenderRequest): RenderPlanRequest`, per the proposal, and
   export it.
4. In `planRender`, make the pass changes in the proposal's table:
   - `const c1 = toBar + (request.chunk?.overlapBars ?? 0);`
   - `full = selectHaps(query(scheduleStart, c1))`;
   - `base.mode = request.chunk?.mode ?? 'mix'`;
   - `planPass` builds its requests as one chunk request or as `renderChunkPlans(...).map((p) => chunkRequest(base, p))`;
   - each chunk's schedule is `cutSchedule(full, req.toBar + req.overlapBars)`;
   - `chunkLookback` is computed from the request's numeric `lookbackSec` as today;
   - each pushed chunk carries `info: { events: info.events, stemPairs: info.stemPairs }` only
     when `request.chunk` is set.

   `ChunkWindowPlan` and `RenderPass['chunks']` gain the optional `info`. `PlannedRender`'s
   doc says that `chunks` holds one entry with `info` for a `chunk` request.
5. Delete `planJob`, `BATCH_REUSE_SCHEDULE` and the `reuseSchedule` parameter. Rewrite the
   module doc: `planRender` plans every chunk of a render in one call, or one chunk for the
   harness (`chunk`).
   - Keep `evaluateSong` and `querySong`: the page's `render` and `schedule` kinds and the
     host's `query` and `inspect` use them.
   - Keep `PlanJobError`, its name included, because its kinds are the host's wire kinds.
     Renaming it is out of scope.

**Why the job text cannot change.**

- `planEngineJob` reads only `mode` and `sampleRate` from the request (`engine-job.ts:315`,
  `:323`, `:364`). `windowPlan` reads the bars, overlap, sample rate and mode. Lookback reaches
  both only through the resolved `lookbackSec` argument, exactly as in `planJob`. So resolving
  `'auto'` before `planWindow` changes no byte.
- The cut of `selectHaps(query(0, c1))` at `c1` keeps every hap, because every selected onset
  is `< c1`. It keeps the same objects and every error, because each `bar` is `< c1`.
- `frozenSchedule` is false without a preset. `scheduleStart` is 0. The tail is 0.

## Step 2: `pageJobInfo` (TS)

In `src/lib/render/page-job.ts`, add `pageJobInfo(planned: PlannedRender): PageJobInfo`. It
takes a type-only import from `./job-plan`.

- When `planned.chunks.length !== 1` or the chunk has no `info`, it throws ``Error(`not a
  single-chunk plan (${planned.chunks.length} chunk(s))`)``.
- Otherwise it returns:
  - `jsonLength: chunk.json.length`;
  - `events: chunk.info.events`;
  - `errors: planned.errors`;
  - `plan: chunk.plan`;
  - `cps: planned.cps`;
  - `structure: planned.structure`;
  - `stemPairs: chunk.info.stemPairs`.

## Step 3: the render page and the bundle (TS)

1. **`src/render/main.ts`.** The `job` kind becomes:

   ```ts
   const planned = await planRender(rt, request.code, singleChunkRequest(request), captured, onPhase);
   jobJson = planned.chunks[0]!.json;
   jobInfo = pageJobInfo(planned);
   ```

   Drop the `planJob` import, and update the module doc ("plans the chunk's engine job with
   the shared planner, one-chunk request").
2. **`src/host/main.ts`.**
   - Delete `runPlanJob`, `StrudelandHost.planJob`, the `strudelandHost.planJob` member and
     every mention of `__strudelandJob`.
   - The module doc lists `{ planRender, query, inspect }`.
   - `hostAdapter`'s doc says that `planRender` does not wrap `evaluate`.

## Step 4: the host (Rust)

1. **`crates/strudeland-host/src/request.rs`.**
   - Delete `RenderRequest` with its `validate`, `to_json` and tests.
   - Add:

     ```rust
     /// One chunk of a window, for the harness (`SingleChunk` in `job-plan.ts`); the bundle
     /// validates its ranges.
     #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
     #[serde(rename_all = "camelCase", deny_unknown_fields)]
     pub struct SingleChunk {
         pub overlap_bars: u32,
         pub lookback_sec: LookbackSec,
         pub mode: Mode,
     }
     ```

   - `RenderPlanRequest` gains
     `#[serde(default, skip_serializing_if = "Option::is_none")] pub chunk: Option<SingleChunk>`,
     with the doc "plan exactly one chunk of a window (the harness's request); production never
     sets it".
   - Update the module doc.
   - `MAX_BARS` and `SAMPLE_RATE` stay: the render crate and the joiner use them.
   - Tests:
     - `RenderRequest` tests are deleted;
     - the `LookbackSec` round-trip test stays;
     - add `a_single_chunk_parses_and_serializes_in_the_bundle_shape` (the exact JSON, `"auto"`
       and a number);
     - add `a_single_chunk_with_an_unknown_field_is_rejected`;
     - the existing `the_render_plan_json_is_the_bundle_shape_without_the_timeout` gains a
       chunk case.
2. **`crates/strudeland-host/src/planned.rs`.**
   - `PlannedChunk` gains
     `#[serde(default, skip_serializing_if = "Option::is_none")] pub info: Option<ChunkInfo>`.
   - Add `ChunkInfo { pub events: serde_json::Value, pub stem_pairs: u32 }` (`camelCase`).
   - A unit test checks that a chunk with `info` round-trips and a chunk without it serializes
     without the key.
3. **`crates/strudeland-host/src/host.rs`.**
   - Delete `PlannedJob`, `plan_job`, `plan_job_with_timeout`, `plan_job_timed` and
     `Work::Plan`, with its `serve` arm.
   - `Work::PlanRender` gains `profile: bool`.
   - `serve` returns `(PlannedRender, PlanTimings)`. `load` comes from `load_bundle`, `marks`
     from `engine.take_marks()`, and `json` is the time from the drain to the parsed plan.
   - Add `pub(crate) fn plan_render_timed(&self, code, request, profile) -> Result<(PlannedRender, PlanTimings), HostError>`.
     The public `plan_render` calls it with `profile: false` and drops the timings.
   - `PlanTimings`'s doc names `plan_render`.
   - The `plan_render` doc keeps its timeout and error sentences and drops "match
     `Host::plan_job`".
4. **`crates/strudeland-host/src/lib.rs`.**
   - Re-exports drop `PlannedJob` and `RenderRequest`, and add `SingleChunk` and `ChunkInfo`.
   - The crate doc says "plans engine jobs (schema 2)".
5. **`crates/strudeland-host/src/bench.rs`.**
   - `request(bars)` returns `RenderPlanRequest` with:
     - `length: Window { from_bar: 0, to_bar: bars, layout: Linear }`;
     - `chunk: Some(SingleChunk { overlap_bars: 0, lookback_sec: Seconds(0.0), mode: Mix })`;
     - everything else default.
   - `one_run` calls `plan_render_timed` and takes the job from `planned.chunks[0].json`. A
     plan without exactly one chunk is an error string.
   - The module doc names `plan_render`.
   - `the_request_is_the_linear_full_window` asserts the new shape.
6. **`crates/strudeland-host/src/bin/strudeland-host.rs`.**
   - Delete the `plan` arm, `run_plan`, `read_request` and `parse_timeout`. Also delete the
     `MAX_TIMEOUT_SEC`, `timeout_from_secs` and `RenderRequest` imports if they are now
     unused.
   - `USAGE` and the module doc lose `plan`. The doc says:
     - `plan-render` with `chunk` plans one chunk;
     - `timeoutSec` bounds the planning (past it the host error is `timeout: …`, exit 1);
     - `plan` is no longer a command.
7. **Every `RenderPlanRequest { … }` literal** in the workspace gains `chunk: None`. Find them
   all with `git grep -n 'RenderPlanRequest {' -- crates`; there are 29 at `a8151213`. They
   include `strudeland_render::plan`, which never sets one: `RenderOptions` has no chunk.

## Step 5: the harness (TS scripts)

1. **`scripts/render/host-planner.ts`.**
   - `HostPlanner.job(req)` writes the song file, runs `hostPlanRenderArgs(song,
     JSON.stringify(singleChunkRequest(req)))`, and parses with `parsePlanRenderOutput`. It
     returns:
     - `text: planned.chunks[0].json`;
     - `info: pageJobInfo(planned)` (a throw there is `HarnessError('render-failed')`, with
       the message);
     - a lazily parsed `job`, with the same "not JSON" and "not an object" failures as today's
       `parsePlanOutput`.
   - Delete `hostRequestJson`, `hostPlanArgs` and `parsePlanOutput`.
   - Rewrite the module doc's outcomes paragraph for `plan-render`.
   - `parsePlanRenderOutput` keeps its checks.
2. **`scripts/render/job-source.ts`.** The doc says that the host source is one single-chunk
   `plan-render` process per job.
3. **`src/lib/render/host-parity-lib.ts`.**
   - Add `batchRequest(window): RenderPlanRequest`. `batchRequestJson` becomes
     `JSON.stringify(batchRequest(window))`, the same text as today.
   - Add `withoutErrors(p: PlannedRender): Omit<PlannedRender, 'errors'>`, with tests.
4. **`scripts/render/host-parity.ts`.**
   - The module doc describes the new comparisons (proposal, "Callers").
   - Non-batch `compare` keeps its request set and byte comparison. Both sides are now
     single-chunk `planRender`.
   - `compareBatchFixture` keeps today's per-chunk comparison (`compareBatch`) and adds the
     whole-plan comparison:
     - `page.planRender(code, batchRequest(window))` against the host's `planBatch`;
     - each through `withoutErrors`, then `firstDifference`;
     - a difference is `DIFF`, written as `batch-plan.page.json` and `batch-plan.host.json` in
       the fixture's folder, with the line `batch plan: first difference at <path>`.
   - The page-fails/host-fails rules stay as they are.
   - The host's side has been through the Rust mirror (`PlannedRender` keeps no unknown
     field). So a whole-plan difference in a key the mirror lacks is a mirror gap, not a
     planner difference. One example is a `structure` key that only the TS has.
     `--mixed-meter` already compares whole plans and passes, so none is expected.
     - If one shows up, **stop** and report it.
     - Adding the key to the mirror would change the `--info` files, which the A/B guard
       forbids. Leaving it out of the comparison is a decision for the user.
5. **`AGENTS.md`'s `host:parity` row and `CONTRIBUTING.md`** need no new command: the flags do
   not change.

## Step 6: tests (replace, don't layer)

Follow the proposal's test table. In particular:

- **`tests/render-job-plan.test.ts`.**
  - The test at `:839-885` becomes "a single-chunk request at chunk k's bars and lookback,
    overlap 0, equals planRender chunk k", over the same five `cases`. For each chunk k of the
    multi-chunk plan, it builds `singleChunkRequest(chunkRequest(base, plan_k))` and plans it
    on a fresh fake runtime. It then checks:
    - `json` equals `got.chunks[k].json`;
    - its `chunks[0].plan` equals `got.chunks[k].plan`;
    - `Object.is` holds on `lookbackSec`.
  - The fallback test (`:887-899`) is deleted.
  - The three `planJob` describes (success, query reuse, failures) are rewritten onto
    `planRender(rt, CODE, singleChunkRequest(req), captured)`:
    - they read `pageJobInfo(...)` and `chunks[0].json`;
    - `expected(...)` keeps building the want from `planEngineJob`;
    - the query-count assertions still hold: one query when `c0 === 0`, else two.
  - Add a `validateRenderRequest` block with the six `chunk` refusals from the step 1 table
    and an accepted chunk for each mode.
- **`tests/host-entry.test.ts`.** The `planJob:` tests (`:85-187`) go through
  `strudelandHost.planRender(code, JSON.stringify({ ...singleChunkRequest(req) }))`.
  - Tests that re-check what an existing `planRender:` test checks are deleted:
    - the ASCII text;
    - a bad request is kind plan;
    - meta, eval and plan carry their kinds.
  - The console-capture and shim tests are ported. Their subject is the bundle, not the entry
    point.
  - "importing the entry installs" asserts the three members.
- **`tests/render-host-planner.test.ts`.**
  - The `hostRequestJson`, `hostPlanArgs` and `parsePlanOutput` blocks are replaced by
    `HostPlanner.job` tests over a fake runner:
    - the exact argument list and request JSON;
    - exit 1 and 2, a signal, and 2000-character stderr truncation as `render-failed`;
    - a plan with two chunks or no `info` as `render-failed`;
    - the lazily parsed job.
  - The `parsePlanRenderOutput` block stays.
- **New `pageJobInfo` tests** in `tests/render-page-job.test.ts`.
- **`crates/strudeland-host/tests/`.**
  - `host.rs`:
    - `every_corpus_fixture_plans_a_valid_engine_job` uses `plan_render` with a chunk;
    - the deterministic-twice, two-threads and shared-host tests use `plan_render`;
    - the bad meta, throwing song, `Math.random`, info and core-log tests read `plan.errors`
      and `chunks[0].info`;
    - delete `a_request_does_not_see_the_previous_ones_globals`,
      `a_song_waiting_on_a_timer_is_unsettled` and `an_invalid_request_is_plan`. Their
      `plan_render.rs` twins stay. Move any `an_invalid_request_is_plan` case the twin lacks
      into it.
  - `plan_render.rs`: rename and rewrite `chunks_equal_plan_job_of_each_chunk_request` as
    `chunks_equal_the_single_chunk_plan_of_each_chunk_request`.
  - `bin.rs`:
    - the `plan_*` tests move to `plan-render`, for the behaviours its existing tests lack:
      `--request` as a file, `--out`, exit 1 naming the kind, exit 2 with the usage for a bad
      `chunk`, and `timeoutSec` bounding a spinning song (from
      `plan_timeout_bounds_the_planning`);
    - add `plan_is_not_a_command`;
    - delete the rest.
  - `never_panics.rs`: `odd_request_numbers_are_errors` gains the odd `chunk` values from the
    proposal. A bad `chunk` value must be a bad request.
  - `bench.rs`: the hash and size test compares with `plan-render`'s `chunks[0].json`.
  - `perf_budget.rs`: `Fixture.request` is the single-chunk `RenderPlanRequest`. `largest`
    and the timed runs use `plan_render`.
  - `common/mod.rs`: `request(to_bar)` returns the single-chunk request.
- **`crates/strudeland-render/tests/joiner_identity.rs`.** Each per-chunk plan is
  `Host::new().plan_render(&code, &single(plan))`, where `single` builds the chunk request at
  `plan.from_bar..plan.to_bar`, linear, overlap 0, `Seconds(plan.lookback_sec)`, mix. The test
  asserts `chunks.len() == 1` and compares `chunks[0].json` with the batch's. The rest is
  unchanged. The module doc says "a single-chunk `plan_render` per chunk".

## Step 7: docs

- **`AGENTS.md`.**
  - Command table:
    - `render:check`: `--planner host` "plans each engine job with one single-chunk
      `plan-render` of the headless host";
    - `host:parity`: "Compare render-page and headless-host plans (…). Each request is a
      single-chunk `planRender` on both sides, compared by job text byte for byte. `--batch`
      compares each corpus window's whole `plan-render` plan (without `errors`) and each
      chunk's job against the page's single-chunk job. `--mixed-meter` …";
    - the `strudeland-host` row: remove `plan`, and describe `plan-render` as "every chunk of a
      render in one call, or one chunk with `chunk`; `timeoutSec` bounds it";
    - the `perf_budget` row: "the largest corpus fixture's full window, planned as one chunk".
  - Layout: in "The headless host", `planJob` becomes `planRender`, and the binary has
    `plan-render`, `query` and `bench`. In "The render pipeline", `BATCH_REUSE_SCHEDULE` is
    removed.
  - Rules, "Headless host": "`host:parity --batch` is the check that `plan-render` plans the
    page's plans and the page's single-chunk jobs".
- **`crates/strudeland-host/README.md`.**
  - The library and binary lists drop `plan_job`, `PlannedJob` and `plan`.
  - "Batch planning":
    - each chunk's selection is the full schedule cut at the chunk's end;
    - `chunk` plans one chunk for the harness;
    - the `BATCH_REUSE_SCHEDULE` paragraph is deleted.
  - "Planning timeout": `RenderPlanRequest.timeoutSec` only.
  - "Performance budget": bench and `perf_budget` plan one chunk through `plan_render`.
  - "Known differences": `plan_render`'s chunk jobs are byte-equal to the single-chunk plan of
    each chunk's request (`host:parity --batch`).
- **`crates/strudeland-render/README.md`.** Nothing, unless it names `plan_job`. Check with
  `git grep -n plan_job -- '*.md'`.
- `SKILL.md`, `CONTRIBUTING.md` and `GLOSSARY.md` do not change.

## Step 8: regeneration and the full gate

1. Run `bun run format` and `cargo fmt --all`.
2. Run the full gate in [000-prompt.md](000-prompt.md), steps 1-4 and 6-13 (step 5 does not
   apply). For this stage:
   - **Step 2** (`engine:refs`, check). Every micro case and fixture job is planned by the
     page's new `job` kind. The diff summary must show provenance only. A job-text difference
     is a **stop**: the single-chunk path is not `planJob`.
   - **Step 4** must print `provenance-only`.
   - **Step 10** must be all `ok`: `host:parity`, `--batch` (per-chunk and whole-plan) and
     `--mixed-meter`.
   - **Step 11** (`perf_budget`). Record the median. Over `BUDGET.full_song_max_ms` (2000) is
     a **stop**.
   - **Step 12**: `equal: 77 cases`, with no new `contentFrames` beyond stage 001's.
   - **Step 13**: `equal`. The kept engine job texts of the page and host modes are the
     absolute job-text check against `a8151213`.
3. Confirm that the deletions are complete:

   ```sh
   git grep -n 'planJob\|plan_job\|PlannedJob\|BATCH_REUSE_SCHEDULE\|reuseSchedule\|__strudelandJob\|hostPlanArgs\|parsePlanOutput' -- src scripts crates tests AGENTS.md CONTRIBUTING.md
   ```

   It prints nothing except the generated `crates/strudeland-host/js/host.js` if a string
   survives there. It must not, after `host:build`.
4. Check `strudeland-host plan`: `cargo run -p strudeland-host -- plan --song x --request '{}'`
   exits 2 with `unknown command plan`.

## STOP conditions

Stop and report to the user, changing nothing to absorb it, on any of these:

- any job-text difference: in `engine:refs` beyond provenance, in `host:parity`, or in the
  kept jobs of `reports-check`;
- any `ab.sh check` or `reports-check` difference;
- `perf_budget` over budget;
- a need to change `SKILL.md`, a CLI flag, a threshold, a golden or a ref by hand;
- a production caller that would need to set `chunk`.

## Commit message

```
refactor(render): plan the harness's single chunks with planRender and delete planJob

A RenderPlanRequest can now ask for exactly one chunk of a window (overlap,
lookback and mode), planned by the same pass code as every production chunk.
The render page's job kind, the host bundle, the host planner, the bench, the
performance budget and host:parity use it. planJob, BATCH_REUSE_SCHEDULE and
its fallback, Host::plan_job*, PlannedJob, RenderRequest and the
strudeland-host plan subcommand are gone; plan-render's timeoutSec keeps the
planning timeout. host:parity --batch also compares whole plans. Job texts
are unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
