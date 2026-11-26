# 0124 implementation entry point

- **Branch:** `llm/andrew/strudeland/0124-one-planner`.
- **Plan:** [the proposal](../0124-one-planner.md). It is the contract. It covers:
  - the frame fields;
  - the single-chunk request and the pass code;
  - the callers;
  - the tests that are replaced;
  - the A/B guard;
  - the rejected options and the decisions.
- **Base:** `a8151213`, which is `origin/main`. Every diff and check names this hash, because the
  local `main` is stale. **Never diff against `main`.**
- **Evidence:** [reverify.md](reverify.md), with file and line at `a8151213`: the two planners,
  the frame recomputes, the frame-equality proof and what full unification would change. Read
  it before you edit anything.
- **Background reading:**
  - in `AGENTS.md`, the "Render harness", "Rust engine", "Headless host" and "Render pipeline"
    rules, and the command table rows for `render:check`, `engine:refs`, `host:parity`,
    `render:vectors` and the `strudeland-host` binary;
  - `crates/strudeland-host/README.md`, the sections "Batch planning", "Planning timeout",
    "Performance budget" and "Known differences";
  - `crates/strudeland-render/README.md`, its opening pipeline list.

## Stages

The stages run in order. Each one is one commit, and each is green with every gate below before
the next starts.

1. [001-frames.md](001-frames.md):
   - the A/B baseline, recorded **before any edit**;
   - every frame count emitted by the planner, and required in TS and Rust;
   - the recomputes deleted;
   - `batch-window.ts` reading frames from `--info`;
   - per-chunk frames in `joiner.json`.
2. [002-one-planner.md](002-one-planner.md):
   - `RenderPlanRequest.chunk` and the single-chunk pass;
   - `planJob`, `BATCH_REUSE_SCHEDULE`, `Host::plan_job*`, `PlannedJob`, `RenderRequest` and
     `strudeland-host plan` deleted;
   - the page, `HostPlanner`, bench, `perf_budget` and `host:parity` moved onto
     `planRender`.
3. [003-batch-adapter.md](003-batch-adapter.md):
   - `Renderer.renderWindow?`;
   - `PipelineRenderer`;
   - `assembledOf` using it;
   - `CheckDeps.batchWindow` and the `fx.assembled` overwrite deleted.

## Rules for every stage

- **No output change.** These stay byte-identical to the baseline:
  - every engine job text;
  - every render through `strudeland-render` and `strudeland render`;
  - every stem set, exit code and refusal text, and `strudeland render`'s printed summary
    (without its `time:` line);
  - every `render:check` report (normalized) and every job and WAV it keeps.

  The `--info` files may gain `contentFrames` on chunks that lacked it, and nothing else.
- **STOP, and report to the user, on any of these.** Do not work around them:
  - any audio hash difference in `ab.sh check`;
  - any `reports-check` difference;
  - any ref that differs beyond what `ab.sh refs` allows: `provenance.plannerSha256`,
    `index.json`'s file hashes (each must be its file's sha256), and the two measured-floor
    cases' WAVs and `floorDb`; or `engine:refs` failing its check after the write;
  - a `joiner.json` frame count or hash change in `ab.sh vectors`;
  - a need to edit a threshold, a golden, a ref or vector by hand, `SKILL.md`, a CLI flag or
    the A/B script;
  - `perf_budget` over `BUDGET`.

  Never re-record a baseline, and never loosen a check, to absorb a difference.
- **The legacy arithmetic stays.** `c0`, `windowStartFrame`, `frames`, `lookbackSecUsed`,
  `bufferStartSec` and hap timing keep their constant-meter formulas. Only the frame
  *measures* are added (proposal, Out).
- **Diff against `a8151213`, never `main`.**
- **Replace tests, don't layer them.** Each stage file lists what is replaced, what is deleted
  and what is added. A test of deleted code is deleted or ported, never kept beside its
  replacement.
- **New logic ships with tests** in the same commit.
- **Generated files are written only by their commands:**
  - `crates/strudeland-host/js/host.js` and `host.inputs.json`: `bun run host:build`;
  - `crates/strudeland-engine/tests/refs/` and `plans/*/evidence/*.json`:
    `bun run engine:refs --write`;
  - `crates/strudeland-render/tests/vectors/joiner.json`: `bun run render:vectors --write`.

  `src/lib/ipc/bindings.ts` is untouched: no Tauri command changes.
- **Run `bun run format` and `cargo fmt --all`** before each commit.
- **Test cadence.**
  - **While working,** run only targeted tests:
    - one TS file, such as `bun test tests/render-job-plan.test.ts`;
    - one crate, such as `cargo test -p strudeland-render` or `-p strudeland-host`;
    - one fixture, such as `bun run host:parity sine-basic`.
  - **When the stage is done,** run the full gate once, in the order below. Log each command
    and read the log rather than re-running it.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan. A later session sets `S` to its own
scratchpad and restores `$S/0124-baseline/` as [001-frames.md](001-frames.md) step 1 says.
`N` is the stage number (`001`, `002` or `003`). `L` logs a command with its start time and
exit code:

```sh
L() { log="$S/0124-$N-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

Times are from this machine: **m** means measured on earlier tasks, **e** means an estimate.
The baseline log records the real figures for the slow ones.

| Step | Command | Time | Which stages |
|---|---|---|---|
| 1 | `L host-build bun run host:build` | <1 min (m) | all |
| 2 | `L refs-check bun run engine:refs` (expect exit 1, "refs differ": `plannerSha256`, the job files' index hashes and the two floor cases, nothing else) | ~3 min (m) | 001, 002 |
| 3 | `L refs-write bun run engine:refs --write` | ~3 min (m) | 001, 002 |
| 4 | `L refs-ab "$S/0124-baseline/ab.sh" refs` (must exit 0 and end `engine:refs (check) exit 0`; it runs the check mode itself) | ~3 min (e) | 001, 002 |
| 5 | 001: `L vectors-write bun run render:vectors --write`, then `L vectors-ab "$S/0124-baseline/ab.sh" vectors`. 001 and 002: `L engine-vectors bun run engine:vectors` (check mode, must exit 0) | <1 min (e) | 001 (all of it), 002 (`engine:vectors`) |
| 6 | `L bun-test bun test` | ~1 min (m) | all |
| 7 | `L check bun run check` and `L lint bun run lint` | ~1 min each (e) | all |
| 8 | `L cargo-test cargo test --workspace` | ~7 min warm (m); 20+ cold | all |
| 9 | `L clippy cargo clippy --workspace --all-targets -- -D warnings` and `L fmt cargo fmt --all --check` | ~4 min (e) | all |
| 10 | `L parity bun run host:parity`, `L parity-batch bun run host:parity --batch`, `L parity-mm bun run host:parity --mixed-meter` | ~3-5 min each (e); <1 min for `--mixed-meter` (m) | all |
| 11 | `L perf cargo test --release -p strudeland-host --test perf_budget -- --ignored --nocapture` | ~2 min with the release build (e) | 002 (and 001 as a spot check) |
| 12 | `L ab "$S/0124-baseline/ab.sh" check` (must end `equal: 96 cases`) | ~7-12 min with the release build (e) | all |
| 13 | `L reports "$S/0124-baseline/ab.sh" reports-check` (must end `equal`) | ~30-60 min for the four modes (e) | all |

- **Step 2** shows the diff summary before anything is written. Step 4 is the absolute check on
  what step 3 wrote: the JSON refs differ only in `plannerSha256`, `index.json` also only in
  the file hashes (which must match the files) and the floor cases' `floorDb`, no WAV but the
  floor cases' changed, and `engine:refs` (check) passes after the write. The floor cases
  (`vowel-formants`, `mod-lfo-note`) are re-rendered whenever their job bytes change, and
  their Chromium renders differ run to run (`scripts/engine/refs-lib.ts:719`, `:777`).
- **Production job text** (`strudeland_render::plan`'s chunks) is not compared as text by any
  gate. It is guarded through the audio hashes of `ab.sh check`, and in 002 through
  `host:parity --batch` against the page's single-chunk jobs.
- **Step 3 and stage 003.** Stage 003 edits no `PLANNER_FILES`. Its `host:build` changes only
  `host.inputs.json`, because `contract.ts` and `chunks.ts` are host inputs. So steps 2-4 do
  not apply there. If `engine:refs` (step 2) shows any difference in 003, stop.
- **Step 13 is the slowest.** Run it last, once the cheaper gates are green. It needs Chromium
  (`bunx playwright install chromium`).
- **`cargo test --workspace`** needs cmake, a C compiler and the V8 download. The first build
  takes minutes.
- **The gates run on macOS.** `jsmath`'s exact tests are judged there.
- **Each stage report lists:**
  - the exit line of every log;
  - the `ab.sh check` and `reports-check` summaries, and whether `sd` is compared on verdicts
    only (with the varying fields recorded at the baseline);
  - the `ab.sh refs` count;
  - the `perf_budget` median (002).

## Commits

- Use plain Conventional Commits, with the message each stage file gives and no task or plan
  identifiers. End each one with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed. The baseline recovery in 001 step 1 uses
  `git switch --detach` in this worktree, never a second one.

## Follow-ups (the workflow's follow-up rule)

- **Task #151** (the stale `controls.rs` comment in `schedule.ts`) is done in 001. Close it at
  finalization.
- **Task #66** item 1 (`isObject` to `isRecord`) is done in 002. Its note "Not done: removing
  `BATCH_REUSE_SCHEDULE`" becomes "done in this task". Update #66 at finalization; its other
  items stay open.
- File anything deferred with `keitora task new` and `keitora task block <new> 124`. Reference
  the task number here, never in commits or code.
