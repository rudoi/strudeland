# 001: every frame count in the plan

## Goal

- The planner emits every frame measure the joiner needs, for constant meter as well as mixed
  meter:
  - `WindowPlan.contentFrames`, and `overlapFrames` whenever a chunk has overlap;
  - `PlannedRender.musicFrames`;
  - `export.loopStartFrame`, with `loopStartBar`;
  - `repeat.musicFrames`.
- The TS and Rust types make them required.
- Every recompute of `round(bar * sr / cps)` outside the planner is deleted:
  - Rust's joiner and pipeline;
  - `batch-window.ts`;
  - the TS presence checks.

  The only one left is `strudeland_render::testing::window_frames`, a test helper.
- **First, before any edit:** the A/B baseline.

No audio, job text, golden, threshold, CLI flag or `SKILL.md` changes. The `--info` files gain
`contentFrames` on constant-meter chunks, and nothing else.

## Prerequisites

- The tree is at `a8151213` with nothing edited or added under the paths the script guards
  (`GUARDED`): `src`, `crates`, `scripts`, `tests`, `vendor`, `Cargo.toml`, `Cargo.lock`,
  `package.json`, `bun.lock`, `strudel-pins.json`, `vite.render.config.ts` and
  `render/index.html` (`git status`). Changes under `plans/` are fine.
- Chromium is installed for Playwright (`bunx playwright install chromium`), and cmake and a C
  compiler are present.
- Read [the proposal](../0124-one-planner.md) "Stage 001" and "The A/B guard", and
  [reverify.md](reverify.md) "Constant-meter frames from TS equal Rust's exactly" and "One
  harness side effect".

## Step 1: the A/B baseline (before any edit)

`S` and `L` are as in [000-prompt.md](000-prompt.md), with `N=001`. The draft script is
`$S/0124-draft/ab.sh`, written by the planning session and reproduced in full in the appendix
below. If the draft is missing (a new session), write the appendix's text to
`$S/0124-baseline/ab.sh`.

1. Check that the tree is unedited:

   ```sh
   git -C /Volumes/rudenet/code/strudeland-worktrees/poptart diff --quiet a8151213 -- src crates scripts tests vendor Cargo.toml Cargo.lock package.json bun.lock strudel-pins.json vite.render.config.ts render/index.html && echo unedited
   ```

   It must print `unedited`. The script checks again and refuses otherwise.
2. Install the script:

   ```sh
   mkdir -p "$S/0124-baseline" && cp "$S/0124-draft/ab.sh" "$S/0124-baseline/ab.sh" && chmod +x "$S/0124-baseline/ab.sh"
   ```

3. `L baseline "$S/0124-baseline/ab.sh" baseline`. This takes about 15-25 minutes (an
   estimate): a release build, then every case recorded twice. It must end
   `baseline: 96 cases; copied to …/.cache/strudeland-0124`, and list the non-zero exits.
   - The 96 are the 34 corpus windows, 19 cases each for `song-linear.js` and the scratch song
     `frac-737`, 21 for `mixed-meter-loop.js`, and the 3 stem sets.
   - `frac-737` is written by the script to `$S/0124-baseline/songs/frac-737.js`, never under
     `tests/`. It is 137 bpm in 7/8, so a bar is 67598.54 frames and every bar edge after 0
     rounds. It has an intro, a loop section and a tail, and goes through every preset,
     `--loops` and `--section` case.
   - Every `strudeland render` case also records its stdout, without the wall-clock `time:`
     line, so the CLI's printed summary is guarded too.
   - `win-samples` and `win-noise-seed` are expected to exit 1: the engine refuses that
     material.
   - Record any other non-zero exit in the stage report. Such a case stays in the guard: its
     refusal text is guarded too. One example is `stems-late-feedback`, refused by the stem
     buffer budget.
   - If the script says the two recordings differ, a case is not reproducible. **Stop** and
     report it before any edit.
4. `L reports-baseline "$S/0124-baseline/ab.sh" reports-baseline`. This takes about 60-120
   minutes (an estimate): the four `render:check` modes, run twice. It prints each mode's exit.
   - Every run writes into the one folder `$S/0124-baseline/reports/run/`, then is normalized
     and moved, so no output path in a report differs between runs. The normalization drops
     `startedAt`, `engineVersion`, `chromium`, `playwright` and each fixture's `seconds` (wall
     clock, `scripts/render/check.ts:582`).
   - The two runs must be equal. For a Rust mode a difference is a **stop**: report it before
     any edit.
   - If the superdough mode (`sd`) differs between the two runs, but only in measurements and
     not in its exit code or any verdict, the script records the varying fields in
     `reports/base/sd/varies.txt` and prints them. From then on `reports-check` compares `sd`
     on its exit code and its fixture statuses, gate, parity and mutation verdicts only. This
     is a property of our A/B guard, not a repo threshold. Record the varying fields in the
     stage report. The Rust modes stay exact.
   - An `exit 2` is a harness error: Chromium is missing, or the render build failed. Fix the
     environment and re-run this step. The tree is still unedited, so a re-run is still a
     baseline.
   - An `exit 1` (a fixture or mutation failing at `a8151213`) is recorded as it is and must
     stay identical.
5. Note the two logs' durations in the stage report. They replace the estimates in
   `000-prompt.md`'s table for the later stages.

**Recovery.** If `$S/0124-baseline/base` or `reports/base` is lost after edits have started:

- First, copy back the cache. `~/.cache/strudeland-0124/ab.sh` goes to
  `$S/0124-baseline/ab.sh`, `base/` to `$S/0124-baseline/base/`, and `reports-base/` to
  `$S/0124-baseline/reports/base/`.
- If the cache is gone too, rebuild from the unedited tree in this worktree:
  1. `git add -A && git commit -m WIP` (a WIP commit, never a stash);
  2. `git switch --detach a8151213`;
  3. `ab.sh baseline` and `ab.sh reports-baseline`;
  4. `git switch -`, then `git reset --soft HEAD~1` and `git restore --staged .`.

  Never use a scratch worktree.

## Step 2: TypeScript

1. **`src/lib/render/plan.ts`.**
   - Import `renderWindowFrames` from `./timing`.
   - In `windowPlan`, after `c1` and its empty-window check, compute both measures once,
     outside the branch:

     ```ts
     const contentFrames = renderWindowFrames(req.fromBar, req.toBar, cps, req.sampleRate, timing);
     const overlapFrames = req.overlapBars
       ? renderWindowFrames(req.toBar, c1, cps, req.sampleRate, timing)
       : undefined;
     ```

   - Both returns spread
     `contentFrames, ...(overlapFrames === undefined ? {} : { overlapFrames })`.
   - In the timing branch, `to` fed only `contentFrames` and `overlapFrames`, so it goes.
     `start`, `from` and `end` stay, because they feed `frames` and the window start. The
     hoisted expressions are the same numbers: `renderWindowFrames` with a timing is
     `frameAt(to) - frameAt(from)`.
   - Leave `c0`, `lookbackSecUsed`, `windowStartFrame`, `bufferStartSec` and `frames` exactly
     as they are, in both branches.
   - `WindowPlan` gets `contentFrames: number`, whose doc is "mapped musical content frames in
     the request's range, excluding lookback and overlap; every planner sets it". It also
     gets `overlapFrames?: number`, whose doc is "present exactly when `overlapBars > 0`".
2. **`src/lib/render/job-plan.ts`.**
   - `PlannedRender.musicFrames: number` becomes required.
   - `export.loopStartFrame?: number` gets the doc "present exactly when `loopStartBar`
     is".
   - `RenderPass.musicFrames: number` becomes required. That type is `repeat`'s type.
   - In `planRender`'s return, `musicFrames: musicalFrames` is always present.
   - `...(loopStartBar === undefined ? {} : { loopStartBar, loopStartFrame: framesBetween(fromBar, loopStartBar) })`
     replaces the two spreads.
   - `repeat` carries `musicFrames: bodyFrames` always.
   - Fix the module comment if it mentions mixed-meter-only frames.
3. **`src/lib/render/contract.ts`.**
   - `RenderResult.contentFrames: number` becomes required, with the doc "mapped frames between
     fromBar and toBar, excluding lookback and overlap (the plan's `contentFrames`)".
   - `overlapFrames?: number` gets the doc "present exactly when `request.overlapBars > 0`".
4. **`src/lib/render/chunks.ts`.**
   - Delete `contentFrames(r)`. `contentOf`, `assembleWindow`'s `sizes` and the offsets read
     `r.contentFrames`.
   - `overlapOf` works like this:
     - `const { overlapBars } = r.request`;
     - when it is 0 the length is 0;
     - otherwise it is `r.overlapFrames`, and a `RangeError`
       ``overlapOf: no overlapFrames for ${overlapBars} overlap bar(s)`` when that is absent.
   - `assembleWindow`:
     - `bufferStartSec: first.bufferStartSec + first.lookbackSecUsed`;
     - `contentFrames: frames`;
     - no branch.
   - Drop the `windowFrames` import if nothing else uses it.
5. **`src/lib/render/features.ts:137`.** `const n = result.contentFrames;`. Drop the import if
   it is unused.
6. **`src/lib/render/batch-window.ts`.**
   - `BatchInfo.chunks` becomes `{ fromBar: number; toBar: number; contentFrames: number }[]`.
   - `parseBatchInfo` requires `contentFrames` to be a non-negative integer. The message is
     "a chunk window with fromBar, toBar and contentFrames".
   - `batchSeamSecs(info)` is the running sum of `contentFrames` before each chunk after the
     first, over `SAMPLE_RATE`. It no longer takes `timing`.
   - In `batchAssembled`:
     - `frames` is the sum of `info.chunks[].contentFrames`;
     - `contentFrames: frames`;
     - `bufferStartSec` stays `timing ? timing.frameAt(fromBar, SAMPLE_RATE) / SAMPLE_RATE : fromBar / info.cps`;
     - `timing` stays a parameter for that alone, and the module doc says so.
   - The module doc says that the frames and seams are the planner's, read from the info.
7. **`src/lib/render/schedule.ts:166`.** The comment becomes "Numeric controls as read by the
   engine (`coerce.rs` `js_to_number`). Only reverb also converts arrays through their string
   representation (`js_to_number_full`)" (task #151). Nothing else in the file changes.
8. **`src/render/main.ts:208`** becomes `contentFrames: plan.contentFrames`. Keep the
   `overlapFrames` spread.
9. **`scripts/render/rust-engine.ts:514`** becomes `contentFrames: info.plan.contentFrames`.
   Keep the `overlapFrames` spread.
10. **`scripts/render/loudness-vectors.ts`.**
    - `chunkResult` sets `contentFrames: windowFrames(chunk.fromBar, chunk.toBar, cps, SAMPLE_RATE)`.
    - When `chunk.overlapBars > 0` it also sets
      `overlapFrames: windowFrames(chunk.toBar, chunk.toBar + chunk.overlapBars, cps, SAMPLE_RATE)`.
    - `joinerCases` writes each chunk with its `contentFrames` and its buffer `frames` (the
      `chunkResult` frame count):
      `chunks: c.chunks.map((ch, k) => ({ ...ch, contentFrames: …, frames: … }))`.
    - The file's `schema` stays 1 (proposal, "Stage 001").
    - Update the module doc's description of `joiner.json`.
11. **`scripts/render/probes/stem-twin.probe.ts`.** Make it typecheck. It reads
    `windowStartFrame` only, so it probably needs nothing.

## Step 3: Rust

1. **`crates/strudeland-host/src/planned.rs`.**
   - `PlannedRender.music_frames: u64`, `PlannedRepeat.music_frames: u64` and
     `ChunkWindow.content_frames: u64` become required, and always serialized. Their
     `#[serde(default, skip_serializing_if)]` attributes go.
   - `ChunkWindow` gains `overlap_frames: Option<u64>` (`default`, skipped when `None`), with
     the doc "frames after the content, present exactly when the chunk renders overlap".
   - `PlannedExport.loop_start_bar` and `loop_start_frame` stay `Option`s, and their docs say
     "present together".
   - The module's `PLANNED` test text gains `"musicFrames":1058400` and the chunk's
     `"contentFrames":1058400`.
   - `a_wrong_shape_is_rejected` gains a missing `musicFrames` and a missing `contentFrames`.
   - `the_bundle_shape_parses_exactly` asserts the two values and that they re-serialize.
2. **`crates/strudeland-render/src/join.rs`.**
   - Delete `content_frames()`. `kept_range(plan)` and `kept(plan, channels)` read
     `plan.content_frames` and take no `cps`.
   - Remove `Joiner.cps` and every `cps` parameter.
   - `pub fn new(window: &PlannedWindow, spans: &[(Span, u64)], music_frames: u64, tail_frames: u64)`
     is today's `with_tail`, with the music-frames check made unconditionally (no `if let
     Some`).
   - `pub fn for_plans(window, plans: &[ChunkWindow], channels, music_frames: u64, tail_frames: u64)`
     replaces both `for_plans` and `for_plans_with_tail`.
   - Update the module doc: the joiner ports `contentOf` and the concatenation; the frame
     counts are the planner's.
   - Move `window_frames` and `frame_of` to `crates/strudeland-render/src/testing.rs`, both
     `pub`, with their doc comments. Move the `window_frames_matches_the_ts_rounding` test
     with them, into `testing.rs`'s own `#[cfg(test)] mod tests`.
   - The remaining `join.rs` tests use explicit `content_frames` and pass `music_frames` as
     the sum.
   - `join_with_a_bad_cps_is_an_error_not_a_panic` has no `cps` left to be bad. Replace it
     with `joiner_rejects_music_frames_that_differ_from_the_chunks`, the check that is now
     unconditional. Don't keep both.
3. **`crates/strudeland-render/src/pipeline_audio.rs`.**
   - `music_frames` is the plan's.
   - The loop start works like this:

     ```rust
     let loop_start = match export.as_ref().map(|e| (e.loop_start_bar, e.loop_start_frame)) {
         None | Some((None, None)) => None,
         Some((Some(bar), Some(frame))) => Some((bar, frame)),
         Some((Some(_), None)) => return Err(RenderError::render("the plan's loop start has no frame")),
         Some((None, Some(_))) => return Err(RenderError::render("the plan's loop start frame has no bar")),
     };
     ```

   - The repeat check becomes `if repeat.music_frames != expected`.
   - The two `Joiner::for_plans_with_tail` calls become `Joiner::for_plans(…, music_frames,
     tail)`, and `(…, expected, repeat.tail_frames)` for the repeat.
   - Tests:
     - `missing_frames_fall_back_to_the_window` is deleted;
     - `a_half_loop_start_is_an_error` (both halves) replaces it;
     - the `planned()` and `isolated_export()` helpers take `u64`s and `Some` pairs;
     - `isolated_body_errors` drops its `None` frame cases, which can no longer occur, and
       keeps the mismatched-length case.
4. **`crates/strudeland-render/src/render.rs`.** The test helper `plans(n)` sets
   `content_frames: 1`. `joiner(n)` uses `Joiner::for_plans(&window, &plans(n), 2, n as u64, 0)`.
5. **`crates/strudeland-render/src/characterization.rs`** imports `crate::testing::window_frames`.
6. **`crates/strudeland-render/tests/pipeline.rs`** imports
   `strudeland_render::testing::window_frames`.
7. **`crates/strudeland-render/tests/joiner_vectors.rs`.**
   - The `Chunk` struct gains `content_frames: u64` and `frames: u64`.
   - The `ChunkWindow` is built from them: `content_frames`, `frames` and
     `overlap_frames: None`. No frame computation is left in the loop.
   - `Joiner::for_plans(&window, &plans, 2, case.frames, 0)`.
   - Keep the `testing::window_frames(window) == case.frames` assertion.
   - Update the module doc.
8. **`crates/strudeland-render/tests/joiner_identity.rs`.** `Joiner::for_plans(…,
   audio.music_frames, 0)`. Stage 002 rewrites the rest.

## Step 4: tests (replace, don't layer)

| File | Change |
|---|---|
| `tests/render-offline.test.ts` (the `windowPlan` tests) | The expected constant-meter plans gain `contentFrames`, and `overlapFrames` where the case has overlap. Add one case: a constant-meter plan's `contentFrames` and `overlapFrames` equal `windowFrames` of the same edges, at a fractional bar length (cps `97/240`) and at `fromBar > 0`. |
| `tests/render-job-plan.test.ts` | The constant-meter `planRender` tests assert the following. `musicFrames` equals `windowFrames(window)`. A loop export's `export.loopStartFrame` equals `windowFrames(fromBar, loopStartBar)`, and its `repeat.musicFrames` equals `windowFrames(loopStartBar, toBar)`. The sum of the chunks' `contentFrames` equals `musicFrames`, in the existing "planRender chunks" and "loop export windows and decay" blocks. The mixed-meter tests (`:1475-1600`) keep their values. |
| `tests/helpers/render-results.ts` | The fake result sets `contentFrames: windowFrames(fromBar, toBar, cps, SR)`, and `overlapFrames` when the request has overlap. |
| `tests/render-chunks.test.ts` | `overlapOf` with `overlapBars` 0 is empty, and with overlap but no `overlapFrames` it throws. `assembleWindow`'s `bufferStartSec` is `bufferStartSec + lookbackSecUsed` of the first chunk. Delete any test of the fallback branch: a missing `contentFrames` no longer type-checks. |
| `tests/render-batch-window.test.ts` | Fixtures gain `contentFrames`. `parseBatchInfo` refuses a chunk without it, or with a negative or fractional one. `batchAssembled` and `batchSeamSecs` take frames and seams from it, including the "bars that are not whole frames" case. A WAV whose length is not the chunks' sum is a `RangeError`. |
| `tests/render-rust-engine.test.ts`, `tests/render-check-flow.test.ts` | Their fake infos and results gain `contentFrames`. No behaviour change. |
| `tests/render-vectors.test.ts` | It byte-compares the committed vectors, so it follows `render:vectors --write`. If it pins `joiner.json`'s chunk keys, they gain `contentFrames` and `frames`. |
| Rust | As step 3. |

## Step 5: docs

- **`crates/strudeland-render/README.md`**, pipeline step 3 ("Join"). The chunk contents are
  cut at the planner's `windowStartFrame` and `contentFrames`, and the joiner checks that they
  add up to the plan's `musicFrames`. It recomputes no frame count.
- **`crates/strudeland-host/README.md`**, "Batch planning". The result carries every frame
  measure, for every song: `musicFrames`, each chunk's `contentFrames`, and with a loop
  `loopStartFrame` and `repeat.musicFrames`.
- **`AGENTS.md`**: the `batch-window.ts` bullet in "The render pipeline" layout entry. It
  reads the `--info` file's chunk frames.

## Step 6: regeneration and the full gate

1. Run `bun run format` and `cargo fmt --all`.
2. Run the full gate in [000-prompt.md](000-prompt.md), steps 1-13 (step 11 as a spot check).
   For this stage:
   - **Step 2** must report differences in provenance only. Read its diff summary:
     - `plannerSha256` changes, because `plan.ts`, `job-plan.ts` and `schedule.ts` are
       `PLANNER_FILES`;
     - so do the job files' hashes in `index.json`, because each job file carries its
       provenance;
     - and so do the two measured-floor cases (`FLOOR_CASES`,
       `scripts/engine/refs-lib.ts:719`: `vowel-formants` and `mod-lfo-note`). Their job
       bytes change, so `settleFloorCase` (`:777`) re-renders their WAVs and `floorDb` on
       `--write`, and those differ run to run.

     No job text beyond `plannerSha256` may differ.
   - **Step 4** (`ab.sh refs`) must end `provenance-only: K file(s) … floor WAVs re-rendered:
     … engine:refs (check) exit 0`, with K = every ref job and ref file of every case plus
     `index.json`. It checks:
     - every changed JSON ref equals `HEAD`'s with `plannerSha256` removed;
     - `index.json` equals `HEAD`'s with `plannerSha256`, `files[*].sha256` and the floor
       cases' `floorDb` removed, so every `bytes` is unchanged, and each listed hash is the
       sha256 of the file on disk (the precedent is the "Regeneration" list in
       [0116's proposal](../0116-render-request-cleanup/proposal.md), line 139);
     - no WAV changed except the floor cases';
     - `bun run engine:refs` (check mode) exits 0 after the write, which enforces
       `FLOOR_MAX_DB`. That adds about 3 minutes.
   - **Step 5** (`ab.sh vectors`) must print `joiner.json: only contentFrames and frames
     added`, and `bun run engine:vectors` (check mode) must exit 0.
   - **Step 10**: `host:parity` and `--batch` compare job text, which does not change.
     `--mixed-meter` compares whole plans, which already carried the frames, so it is equal.
   - **Step 12** must end `equal: 96 cases, … files; info files gained contentFrames on M
     chunk(s)`. M is the constant-meter chunk count across the `strudeland-render` cases, and
     is greater than 0.
   - **Step 13** must end `equal`. This is the check on the `assembleWindow` `bufferStartSec`
     side effect.
3. Confirm that no recompute is left:

   ```sh
   git grep -n 'contentFrames ??\|overlapFrames ??\|contentFrames === undefined\|unwrap_or_else(|| join::window_frames\|content_frames(p' -- src scripts crates
   ```

   It prints nothing. The `overlapOf` throw is the only `overlapFrames === undefined` check.

## STOP conditions

Stop and report to the user, changing nothing to absorb it, on any of these:

- any audio hash, exit code or failure stderr difference in `ab.sh check`;
- an `--info` difference other than an added `contentFrames`;
- any `reports-check` difference;
- a ref that differs beyond what `ab.sh refs` allows (`plannerSha256`, `index.json`'s file
  hashes, the floor cases' WAVs and `floorDb`), or `engine:refs` failing its check after the
  write;
- a `joiner.json` frame count or hash that changes;
- a need to edit a threshold, golden, ref, vector or `SKILL.md`;
- a change to `windowStartFrame`, `frames`, `c0`, `bufferStartSec` or `lookbackSecUsed` in any
  plan.

## Commit message

```
refactor(render): carry every frame count in the plan and drop the recomputes

windowPlan emits contentFrames (and overlapFrames with overlap) for constant
meter too, and planRender always emits musicFrames, loopStartFrame and
repeat.musicFrames. The Rust plan types make them required, so the joiner and
the pipeline read the planner's counts instead of recomputing
round(bar * sr / cps); window_frames moves to the test helpers. The batch
window reads its frames and seams from the --info chunks, and joiner.json
carries per-chunk frames. The constant-meter arithmetic for c0, the window
start, the buffer length and hap timing is unchanged, so no job text or audio
changes.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

## Appendix: `ab.sh` as drafted

This is the text of `$S/0124-draft/ab.sh` at planning time. The copy in `$S/0124-baseline/` and
`~/.cache/strudeland-0124/` is authoritative once step 1 has run. Never edit the script after
the baseline is recorded.

```sh
#!/bin/sh
# 0124 A/B guard (draft; the plan's 001-frames.md step 1 copies it to $S/0124-baseline/ab.sh).
#
#   ab.sh baseline          build strudeland-render and strudeland from an unedited tree, record
#                           every case twice (base/, base2/) and require the two to be equal
#   ab.sh check             build both from the working tree, record into cur/, compare with base/
#   ab.sh reports-baseline  run the four render:check modes twice on an unedited tree
#                           (reports/base/, reports/base2/) and require the two to be equal; a
#                           superdough run that varies is recorded and compared on verdicts only
#   ab.sh reports-check     run them on the working tree into reports/cur/ and compare
#   ab.sh refs              after `engine:refs --write`: every changed JSON ref differs from HEAD
#                           only in provenance.plannerSha256 (index.json: also the file hashes,
#                           which must match the files, and the floor cases' floorDb), no WAV
#                           but the floor cases' changed, and `engine:refs` (check) exits 0
#   ab.sh vectors           after `render:vectors --write`: joiner.json differs from HEAD only by
#                           the added per-chunk contentFrames and frames
#
# `check` compares audio exactly: every output file's sha256, every exit code, the stderr of
# every failed case and `strudeland render`'s stdout (its `time:` line removed). An `--info` file
# may differ from the baseline in one way only: a chunk that had no `contentFrames` may gain one.
# `reports-check` compares each normalized report.json (run-dependent keys removed), each exit
# code, and for the Rust modes the sorted sha256 list of every kept engine job text and WAV
# (`--keep-wav`). The superdough mode is compared exactly too, unless reports-baseline found it
# not reproducible: then on its exit code and its fixture, gate, parity and mutation verdicts
# alone. That is a property of this A/B guard, not a repo threshold; the Rust modes stay exact.
set -eu

B=$(cd "$(dirname "$0")" && pwd)
REPO=/Volumes/rudenet/code/strudeland-worktrees/poptart
BASE_SRC=a8151213
CACHE="$HOME/.cache/strudeland-0124"
CORPUS="$REPO/tests/render/corpus"
MIXED="$REPO/tests/render/mixed-meter-loop.js"
# A scratch song, never committed: a bar that is not a whole number of frames.
SONGS="$B/songs"
FRAC="$SONGS/frac-737.js"
# The micro cases whose Chromium reference varies run to run (scripts/engine/refs-lib.ts
# FLOOR_CASES): `engine:refs --write` re-renders their WAVs and floorDb whenever a job changes.
FLOOR_CASES='vowel-formants mod-lfo-note'
# The tree the baselines are recorded from: no edit and no new file under these paths.
GUARDED='src crates scripts tests vendor Cargo.toml Cargo.lock package.json bun.lock strudel-pins.json vite.render.config.ts render/index.html'
# Every case renders here, on both sides, so a path written into an output (stems.json) is the
# same in the baseline and the check.
W="$B/work"
TAB=$(printf '\t')

die() {
	echo "ab.sh: $*" >&2
	exit 2
}

sha() { shasum -a 256 "$1" | cut -d' ' -f1; }

stamp() { date '+%Y-%m-%dT%H:%M:%S'; }

# The tree must be a8151213 for every GUARDED path: no edit, no new file.
require_unedited() {
	# shellcheck disable=SC2086 # GUARDED is a list of paths
	if ! git -C "$REPO" diff --quiet "$BASE_SRC" -- $GUARDED; then
		die "$GUARDED: differ from $BASE_SRC; refusing to record a baseline"
	fi
	# shellcheck disable=SC2086
	if [ -n "$(git -C "$REPO" status --porcelain --untracked-files=all -- $GUARDED)" ]; then
		die "untracked or uncommitted files under $GUARDED; refusing to record a baseline"
	fi
}

# The scratch song: 137 bpm in 7/8 is 3.5 quarters a bar, 67598.54 frames at 44.1 kHz, so every
# bar edge after 0 rounds. An intro, a loop section and a tail, as song-linear.js.
write_songs() {
	mkdir -p "$SONGS"
	cat >"$FRAC" <<'EOF'
// 0124 A/B scratch song: a bar of 67598.54 frames at 44.1 kHz.
const meta = {
  title: 'frac-737',
  bpm: 137,
  meter: '7/8',
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 3, loop: true },
    { name: 'tail', bars: 2 },
  ],
  loops: 2,
};

$: song(meta, {
  intro: note("c3 e3 g3 b3 c4 e4 g4").s('triangle'),
  body: note("<c2 g1 a1>*7").s('sawtooth').lpf(700).room(0.3),
  tail: note("c3").s('supersaw').release(2),
})
EOF
}

target_dir() {
	cargo metadata --no-deps --format-version 1 --manifest-path "$REPO/Cargo.toml" | jq -r .target_directory
}

# build SUFFIX: release strudeland-render and strudeland from the working tree, copied to bin/.
build() {
	mkdir -p "$B/bin"
	(cd "$REPO" && cargo build --release --locked -p strudeland-render -p strudeland-cli \
		--bin strudeland-render --bin strudeland)
	t=$(target_dir)
	cp "$t/release/strudeland-render" "$B/bin/strudeland-render-$1"
	cp "$t/release/strudeland" "$B/bin/strudeland-$1"
}

# cases: one line per case, tab-separated: name, tool (`render` = strudeland-render render,
# `cli` = strudeland render), the song path, then the tool's range and preset flags. The output
# flags are added by `record`.
cases() {
	# Every corpus fixture's window, as `render:check --planner batch` renders it.
	jq -r --arg c "$CORPUS" '.fixtures[] |
		"win-\(.name)\trender\t\($c)/\(.file)\t--from \(.windowBars[0]) --to \(.windowBars[1]) --layout \(.layout)"' \
		"$REPO/tests/render/corpus.json"
	# The three sectioned songs: clip, loop counts, sections, lengths, every preset raw and
	# leveled, a one-shot section, a plain leveled clip and an orbit stem set.
	for song in "song-linear|$CORPUS/song-linear.js|intro body tail|body" \
		"mixed-meter|$MIXED|opening F1 F2 E ending|E" \
		"frac-737|$FRAC|intro body tail|body"; do
		n=${song%%|*}
		rest=${song#*|}
		p=${rest%%|*}
		rest=${rest#*|}
		sections=${rest%%|*}
		loop=${rest#*|}
		printf '%s\t%s\t%s\t%s\n' "$n-clip" render "$p" ""
		printf '%s\t%s\t%s\t%s\n' "$n-loops1" render "$p" "--loops 1"
		printf '%s\t%s\t%s\t%s\n' "$n-loops3" render "$p" "--loops 3"
		for s in $sections; do
			printf '%s\t%s\t%s\t%s\n' "$n-section-$s" render "$p" "--section $s"
		done
		printf '%s\t%s\t%s\t%s\n' "$n-len3" render "$p" "--length 3"
		printf '%s\t%s\t%s\t%s\n' "$n-len11" render "$p" "--length 11"
		for preset in tagged-loop intro-loop folded-loop one-shot; do
			printf '%s\t%s\t%s\t%s\n' "$n-$preset-raw" cli "$p" \
				"--preset $preset --loudness off --format wav --bit-depth 32f"
			printf '%s\t%s\t%s\t%s\n' "$n-$preset-lev" cli "$p" "--preset $preset"
		done
		printf '%s\t%s\t%s\t%s\n' "$n-one-shot-section-$loop" cli "$p" \
			"--preset one-shot --section $loop --loudness off --format wav --bit-depth 32f"
		printf '%s\t%s\t%s\t%s\n' "$n-cli-lev" cli "$p" ""
		printf '%s\t%s\t%s\t%s\n' "$n-stems-orbit" cli "$p" "--stems orbit"
	done
	# The stems fixtures' stem sets (no sections: a length).
	printf '%s\t%s\t%s\t%s\n' stems-stems-5 cli "$CORPUS/stems-5.js" "--stems orbit --length 8"
	printf '%s\t%s\t%s\t%s\n' stems-stems-shared cli "$CORPUS/stems-shared.js" "--stems orbit --length 4"
	printf '%s\t%s\t%s\t%s\n' stems-late-feedback cli "$CORPUS/late-feedback.js" "--stems orbit --length 160"
}

# record SUFFIX OUT: every case with the SUFFIX binaries, into OUT.
record() {
	render="$B/bin/strudeland-render-$1"
	cli="$B/bin/strudeland-$1"
	out=$2
	rm -rf "$out"
	mkdir -p "$out/info"
	: >"$out/audio.tsv"
	write_songs
	cases >"$B/cases.tsv"
	while IFS="$TAB" read -r name tool song flags; do
		rm -rf "$W"
		mkdir -p "$W"
		set +e
		case "$tool" in
		render)
			# shellcheck disable=SC2086 # the flags are words
			"$render" render --song "$song" $flags --loudness off --format wav --bit-depth 32f \
				--jobs 4 --out "$W/out.wav" --info "$W/info.json" </dev/null >/dev/null 2>"$B/stderr"
			;;
		cli)
			case "$flags" in
			*--stems*) dest="$W/stems" ;;
			*) dest="$W/out.wav" ;;
			esac
			# shellcheck disable=SC2086
			"$cli" render "$song" $flags --jobs 4 --out "$dest" </dev/null >"$B/stdout" 2>"$B/stderr"
			;;
		*) die "unknown tool $tool" ;;
		esac
		code=$?
		set -e
		printf '%s\texit\t%s\n' "$name" "$code" >>"$out/audio.tsv"
		if [ "$code" != 0 ]; then
			printf '%s\tstderr\t%s\n' "$name" "$(sha "$B/stderr")" >>"$out/audio.tsv"
		fi
		if [ "$tool" = cli ]; then
			# The CLI's summary, without its wall-clock `time:` line.
			sed '/^  time: /d' "$B/stdout" >"$B/stdout.norm"
			printf '%s\tstdout\t%s\n' "$name" "$(sha "$B/stdout.norm")" >>"$out/audio.tsv"
		fi
		(cd "$W" && find . -type f | LC_ALL=C sort) | while IFS= read -r f; do
			f=${f#./}
			if [ "$f" = info.json ]; then
				jq -S 'del(.summary.timings)' "$W/info.json" >"$out/info/$name.json"
			else
				printf '%s\t%s\t%s\n' "$name" "$f" "$(sha "$W/$f")" >>"$out/audio.tsv"
			fi
		done
	done <"$B/cases.tsv"
	rm -rf "$W" "$B/stderr" "$B/stdout" "$B/stdout.norm"
}

# compare_info BASE CUR: 0 when CUR equals BASE after removing, per chunk, a contentFrames that
# BASE's chunk lacks.
compare_info() {
	jq -ne --slurpfile b "$1" --slurpfile c "$2" '
		$b[0] as $B | $c[0] as $C
		| ($C | .chunks |= [range(0; length) as $i | .[$i]
			| if (($B.chunks[$i] // {}) | has("contentFrames")) then . else del(.contentFrames) end
		  ]) as $C2
		| $B == $C2' >/dev/null
}

# compare BASE CUR: prints `equal …` or the differences; 0 only when equal.
compare() {
	bad=0
	if ! diff "$1/audio.tsv" "$2/audio.tsv" >"$B/diff.txt"; then
		echo "DIFFERENT audio ($(grep -c '^[<>]' "$B/diff.txt") lines, $B/diff.txt):"
		head -40 "$B/diff.txt"
		bad=1
	fi
	for f in "$1"/info/*.json; do
		n=$(basename "$f")
		if [ ! -f "$2/info/$n" ]; then
			echo "DIFFERENT info: $n missing from $2"
			bad=1
		elif ! compare_info "$f" "$2/info/$n"; then
			echo "DIFFERENT info: $n (more than an added contentFrames)"
			bad=1
		fi
	done
	for f in "$2"/info/*.json; do
		[ -f "$1/info/$(basename "$f")" ] || {
			echo "DIFFERENT info: $(basename "$f") is new"
			bad=1
		}
	done
	if [ "$bad" = 0 ]; then
		cases=$(cut -f1 "$2/audio.tsv" | LC_ALL=C sort -u | wc -l | tr -d ' ')
		files=$(awk -F'\t' '$2 != "exit" && $2 != "stderr" && $2 != "stdout"' "$2/audio.tsv" | wc -l | tr -d ' ')
		added=0
		for f in "$2"/info/*.json; do
			k=$(jq '[.chunks[] | select(has("contentFrames"))] | length' "$f")
			b=$(jq '[.chunks[] | select(has("contentFrames"))] | length' "$1/info/$(basename "$f")")
			added=$((added + k - b))
		done
		echo "equal: $cases cases, $files files; info files gained contentFrames on $added chunk(s)"
	fi
	return "$bad"
}

# The four render:check modes: name, then flags.
MODES='sd|
rust-page|--engine rust --keep-wav
rust-host|--engine rust --planner host --keep-wav
rust-batch|--engine rust --planner batch --keep-wav'

# The verdicts of a report: what a superdough run that is not reproducible is compared on.
VERDICTS='{
	fixtures: [.fixtures[] | {name, status, gates: [.gates[] | {gate, verdict}], parity: (.parity.verdict // null)}],
	mutations: [.mutations[] | {name, fixture, gate, status}],
	harnessError: (.harnessError != null)
}'

# reports SIDE: every mode into reports/SIDE/<mode>/ (normalized report, exit code, file hashes).
# Every run writes into the same reports/run/, so no output path differs between sides.
reports() {
	run="$B/reports/run"
	echo "$MODES" | while IFS='|' read -r mode flags; do
		dir="$B/reports/$1/$mode"
		rm -rf "$dir" "$run"
		mkdir -p "$dir"
		echo "$(stamp) render:check $mode"
		set +e
		# shellcheck disable=SC2086
		(cd "$REPO" && bun run render:check $flags --out "$run") </dev/null >"$dir/log" 2>&1
		echo "exit $?" >"$dir/exit"
		set -e
		echo "$(stamp) render:check $mode: $(cat "$dir/exit")"
		# Run-dependent: the start time, the tool versions and each fixture's wall-clock seconds.
		jq -S 'del(.startedAt, .engineVersion, .chromium, .playwright, .fixtures[].seconds)' \
			"$run/report.json" >"$dir/report.json"
		if [ -n "$flags" ]; then
			# Names carry a per-run counter: compare the sorted content hashes, not the names.
			find "$run" \( -name '*.wav' -o -name '*.job.json' \) -type f -exec shasum -a 256 {} + |
				cut -d' ' -f1 | LC_ALL=C sort >"$dir/files.txt"
		fi
		rm -rf "$run"
	done
}

# varying_paths A B: the leaf paths whose values differ between two report.json files.
varying_paths() {
	jq -rn --slurpfile a "$1" --slurpfile b "$2" '
		$a[0] as $A | $b[0] as $B
		| [(($A | paths(scalars)), ($B | paths(scalars))) as $p
			| select(($A | getpath($p)) != ($B | getpath($p)))
			| $p | map(if type == "number" then "[]" else tostring end) | join(".")]
		| unique | .[]'
}

# same_verdicts A_DIR B_DIR: 0 when the exit codes and the report verdicts are equal.
same_verdicts() {
	cmp -s "$1/exit" "$2/exit" &&
		[ "$(jq -cS "$VERDICTS" "$1/report.json")" = "$(jq -cS "$VERDICTS" "$2/report.json")" ]
}

case "${1:-}" in
baseline)
	require_unedited
	echo "$(stamp) build"
	build baseline
	echo "$(stamp) record base"
	record baseline "$B/base"
	echo "$(stamp) record base2 (determinism)"
	record baseline "$B/base2"
	if ! compare "$B/base" "$B/base2"; then
		die "two baseline recordings differ: a case is not reproducible (see above)"
	fi
	mkdir -p "$CACHE"
	cp "$0" "$CACHE/ab.sh"
	rm -rf "$CACHE/base"
	cp -R "$B/base" "$CACHE/base"
	echo "$(stamp) baseline: $(cut -f1 "$B/base/audio.tsv" | LC_ALL=C sort -u | wc -l | tr -d ' ') cases; copied to $CACHE"
	awk -F'\t' '$2 == "exit" && $3 != 0 { print "  exits " $3 ": " $1 }' "$B/base/audio.tsv"
	;;
check)
	[ -d "$B/base" ] || die "no baseline at $B/base (see 001-frames.md step 1)"
	echo "$(stamp) build"
	build current
	echo "$(stamp) record cur"
	record current "$B/cur"
	compare "$B/base" "$B/cur"
	;;
reports-baseline)
	require_unedited
	reports base
	echo "$(stamp) reports base2 (determinism)"
	reports base2
	for m in "$B"/reports/base/*; do
		mode=$(basename "$m")
		if diff -r -x log "$m" "$B/reports/base2/$mode" >"$B/reports-base-diff.txt"; then
			continue
		fi
		if [ "$mode" != sd ]; then
			die "two $mode report runs differ (a Rust mode must be reproducible; $B/reports-base-diff.txt)"
		fi
		if ! same_verdicts "$m" "$B/reports/base2/sd"; then
			die "two superdough report runs differ in an exit code or a verdict ($B/reports-base-diff.txt)"
		fi
		# Kept with the baseline: reports-check compares sd on verdicts only from here on.
		varying_paths "$m/report.json" "$B/reports/base2/sd/report.json" >"$m/varies.txt"
		echo "sd is not reproducible: compared on verdicts only; varying fields in $m/varies.txt:"
		head -20 "$m/varies.txt"
	done
	mkdir -p "$CACHE"
	cp "$0" "$CACHE/ab.sh"
	rm -rf "$CACHE/reports-base"
	cp -R "$B/reports/base" "$CACHE/reports-base"
	for m in "$B"/reports/base/*; do echo "$(basename "$m"): $(cat "$m/exit")"; done
	;;
reports-check)
	[ -d "$B/reports/base" ] || die "no report baseline at $B/reports/base (see 001-frames.md step 1)"
	reports cur
	bad=0
	: >"$B/reports-diff.txt"
	for m in "$B"/reports/base/*; do
		mode=$(basename "$m")
		if [ -f "$m/varies.txt" ]; then
			if same_verdicts "$m" "$B/reports/cur/$mode"; then
				echo "$mode: equal verdicts and exit code (not reproducible: compared on verdicts only)"
			else
				echo "$mode: DIFFERENT verdicts or exit code" | tee -a "$B/reports-diff.txt"
				bad=1
			fi
		elif diff -r -x log "$m" "$B/reports/cur/$mode" >>"$B/reports-diff.txt"; then
			echo "$mode: equal (report, exit code$([ "$mode" = sd ] || echo ', kept jobs and WAVs'))"
		else
			echo "$mode: DIFFERENT"
			bad=1
		fi
	done
	if [ "$bad" = 0 ]; then
		echo "equal: $(ls "$B/reports/cur" | tr '\n' ' ')"
	else
		echo "DIFFERENT ($B/reports-diff.txt):"
		head -60 "$B/reports-diff.txt"
		exit 1
	fi
	;;
refs)
	cd "$REPO"
	bad=0
	n=0
	floor=''
	refs=crates/strudeland-engine/tests/refs
	evidence=':(glob)plans/*/evidence/**'
	added=$(git status --porcelain --untracked-files=all -- "$refs" "$evidence" | grep -v '^ M ' || true)
	if [ -n "$added" ]; then
		echo "refs: files added or deleted:"
		echo "$added"
		bad=1
	fi
	strip='walk(if type == "object" then del(.plannerSha256) else . end)'
	# shellcheck disable=SC2086 # FLOOR_CASES is a list of names
	floors=$(printf '%s\n' $FLOOR_CASES | jq -R . | jq -sc .)
	for f in $(git diff --name-only HEAD -- "$refs" "$evidence"); do
		case "$f" in
		"$refs/index.json")
			# Each job file's hash moves with its provenance, and the floor cases' floorDb with
			# their re-render: drop those; every other key and every `bytes` must be HEAD's.
			istrip="$strip | del(.files[].sha256)
				| reduce \$floors[] as \$c (.; if .cases[\$c] then .cases[\$c] |= del(.floorDb) else . end)"
			if git show "HEAD:$f" | jq -S --argjson floors "$floors" "$istrip" >"$B/refs-old.json" &&
				jq -S --argjson floors "$floors" "$istrip" "$f" >"$B/refs-new.json" &&
				cmp -s "$B/refs-old.json" "$B/refs-new.json"; then
				n=$((n + 1))
			else
				echo "refs: $f differs beyond plannerSha256, the file hashes and the floor cases' floorDb"
				bad=1
			fi
			# Each listed hash must be the file's on disk.
			jq -r '.files | to_entries[] | "\(.key)\t\(.value.sha256)"' "$f" |
				while IFS="$TAB" read -r name want; do
					[ "$(sha "$refs/$name" 2>/dev/null)" = "$want" ] || echo "$name"
				done >"$B/refs-hash.txt"
			if [ -s "$B/refs-hash.txt" ]; then
				echo "refs: index.json hashes that are not the file's:"
				head -20 "$B/refs-hash.txt"
				bad=1
			fi
			;;
		*.json)
			if git show "HEAD:$f" | jq -S "$strip" >"$B/refs-old.json" &&
				jq -S "$strip" "$f" >"$B/refs-new.json" &&
				cmp -s "$B/refs-old.json" "$B/refs-new.json"; then
				n=$((n + 1))
			else
				echo "refs: $f differs beyond provenance.plannerSha256"
				bad=1
			fi
			;;
		*.wav)
			c=$(basename "$f" .wav)
			case " $FLOOR_CASES " in
			*" $c "*) floor="$floor $c" ;;
			*)
				echo "refs: $f changed (only a floor case's WAV may be re-rendered)"
				bad=1
				;;
			esac
			;;
		*)
			echo "refs: $f changed (only JSON provenance and the floor cases may change)"
			bad=1
			;;
		esac
	done
	rm -f "$B/refs-old.json" "$B/refs-new.json" "$B/refs-hash.txt"
	if [ "$bad" = 0 ]; then
		# The written refs are current, and the re-rendered floors are within FLOOR_MAX_DB.
		echo "$(stamp) engine:refs (check)"
		set +e
		bun run engine:refs </dev/null >"$B/refs-check.log" 2>&1
		code=$?
		set -e
		if [ "$code" != 0 ]; then
			echo "refs: engine:refs (check) exited $code after --write ($B/refs-check.log)"
			tail -20 "$B/refs-check.log"
			bad=1
		fi
	fi
	if [ "$bad" = 0 ]; then
		echo "provenance-only: $n file(s) differ from HEAD in plannerSha256 (index.json: and its hashes)" \
			"alone; floor WAVs re-rendered:${floor:- none}; engine:refs (check) exit 0"
	fi
	exit "$bad"
	;;
vectors)
	cd "$REPO"
	f=crates/strudeland-render/tests/vectors/joiner.json
	strip='del(.cases[].chunks[].contentFrames, .cases[].chunks[].frames)'
	git show "HEAD:$f" | jq -S "$strip" >"$B/vec-old.json"
	jq -S "$strip" "$f" >"$B/vec-new.json"
	have=$(jq '[.cases[].chunks[] | select(has("contentFrames") and has("frames"))] | length' "$f")
	all=$(jq '[.cases[].chunks[]] | length' "$f")
	if cmp -s "$B/vec-old.json" "$B/vec-new.json" && [ "$have" = "$all" ]; then
		echo "joiner.json: only contentFrames and frames added ($all chunks); frames and hashes unchanged"
	else
		echo "joiner.json: differs beyond the added per-chunk fields, or a chunk lacks them ($have of $all)"
		exit 1
	fi
	;;
*)
	echo "usage: ab.sh baseline | check | reports-baseline | reports-check | refs | vectors" >&2
	exit 2
	;;
esac
```
