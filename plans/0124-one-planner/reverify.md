# 0124 step 0: re-verification on main a815121

The friction is still there. Two claims in the task are wrong: the job text does not change, and full RenderTiming unification would change audio.

## Two planners

- **`planJob`** is at `src/lib/render/job-plan.ts:201-231`. It plans one window: schedule `[0, toBar+overlap)`, then the `'auto'` lookback over `[0, c1)`, then `planWindow`.
  - The render page's `job` kind (`src/render/main.ts:156`) calls it.
  - So does the host bundle (`src/host/main.ts:192`), which serves Rust `Host::plan_job*` (`crates/strudeland-host/src/host.rs:103-141`) and the bin's `plan` subcommand.
  - Its users are the harness (the `--planner page/host/batch` JobSources), `engine:refs`, `host:parity`, the bench and `perf_budget`.
- **`planRender`** (`job-plan.ts:535-701`) is what production runs. `strudeland render` and the app export both go through `strudeland_render::plan`, then `Host::plan_render`. Live playback and the piano roll do not plan through the host.
- **The gates check `planJob` chunks, production renders `planRender` chunks.** Two things link the two today:
  - `host:parity --batch`;
  - `BATCH_REUSE_SCHEDULE` (`job-plan.ts:289-295`) with the test-only `reuseSchedule` (`:541-542, :626-629`; test at `tests/render-job-plan.test.ts:886-899`).

## Frames only for mixed meter

- **`windowPlan`** (`src/lib/render/plan.ts:126-172`) sets `contentFrames` and `overlapFrames` only in the timing branch (`:143-160`).
- **`planRender`** adds `musicFrames`, `loopStartFrame` and `repeat.musicFrames` only with timing (`:672`, `:682-684`, `:691`).
- **Rust recomputes them** for constant meter:
  - `strudeland-render/src/join.rs:14-30` (`window_frames`, `frame_of`, a `content_frames` fallback, and `Joiner::new`);
  - `pipeline_audio.rs:80-87` (the `music_frames` and `loop_start_frame` fallbacks);
  - `:129`, an optional repeat check.
- **`batch-window.ts`** recomputes them too (`:136-141`, `:157`, `:175`) and hand-parses `--info` (`:34-90`).

## Constant-meter frames from TS equal Rust's exactly

`windowFrames` (`contract.ts:115-121`) is `Math.round(bar*sr/cps)`. `frame_of` (`join.rs:19-21`) is `(bar*SR/cps).round()`: the same operations, with `cps` round-tripped exactly through serde `float_roundtrip`. So the following are what Rust computes today, with no ±1 risk:

- `contentFrames = windowFrames(from, to)`
- `musicFrames = windowFrames(window)`
- `loopStartFrame = windowFrames(from, loopStart)`
- `repeat.musicFrames = windowFrames(loopStart, to)`

## What full unification would change (not done)

- The legacy `round((from-c0)*sr/cps)` differs from the grid `round(from*sr/cps) - round(c0*sr/cps)` in 39,539 of 192,960 cases (20%), always by ±1. The sweep covered bpm 40-240, six meters, sr 44100 and c0 < from. So moving `windowStartFrame` or `frames` onto the grid changes `job.frames` and the cut offset, and with them the audio bytes.
- A degenerate RenderTiming would also change:
  - `hapTiming`, both `t` and the added `phaseSec`;
  - `historyStart`;
  - the pause schedule and the chunk plan;
  - superdough's `offline.ts`.
- So this plan unifies only the frame *measures*. The legacy arithmetic for c0, start, frames, `bufferStartSec` and hap timing stays.

## Artifacts

- **Engine job text does not change.** `page-job.ts:53` serialises only `planned.job`, and the frame fields travel in the plan or the info, never in the job. The job schema stays 2, and the engine never reads these fields. The engine refs change in their provenance hash only (`PLANNER_FILES` includes `plan.ts` and `job-plan.ts`).
- **Rebuilt by the repo's commands:**
  - `host.js` and `host.inputs.json` (`host:build`);
  - refs provenance (`engine:refs --write`);
  - `joiner.json`, if its generator emits per-chunk `contentFrames` (`render:vectors --write`). Its frame counts and hashes stay the same.
- **Unchanged:** `finish.json` (synthetic), the render goldens (the superdough path), `SKILL.md`, CLI flags and output, thresholds, and render length.
- **One harness side effect.** `RenderResult.contentFrames` will always be copied (`rust-engine.ts:514`, `main.ts:208`), so `assembleWindow` takes its mapped branch for constant-meter songs too. On the corpus, the only fixtures that start past bar 0 are late-feedback (from 136) and crowded-bass (from 4). For both, `c0/cps + (from-c0)/cps === from/cps`, so nothing changes.

## Guard

No committed hash covers a corpus song through the whole pipeline. Before any edit, an A/B baseline records the sha256 of release `strudeland-render` output for:

- every corpus fixture window;
- the song fixtures' clip, section, length and every preset;
- `tests/render/mixed-meter-loop.js`;
- the stem sets.

Each stage must reproduce them exactly. The `--info` diffs may only add frame fields.
