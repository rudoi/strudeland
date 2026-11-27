# 0129 implementation entry point

- **Branch:** `llm/andrew/strudeland/0129-planner-sections`.
- **Base:** `ee91dbd`. Diff against it, never against a local `main`, which may be stale. The
  branch commits on top of it before stage 001 are plan documents only.
- **Evidence:** [reverify.md](reverify.md), with file and line at `ee91dbd`. Read §1 (the
  re-derivation and every test that pins a refusal), §2 (the mapping table) and §3 (why the
  transport gives the same names) before you edit anything.
- **Background reading:**
  - `AGENTS.md`: the render pipeline bullet (`:279-301`), the headless host bullet
    (`:261-278`), and the Rules on the Rust engine refs, the headless host and the render
    pipeline;
  - `crates/strudeland-render/src/plan.rs` (all of it), `error.rs:39-47,81-98`;
  - `src/lib/render/job-plan.ts:258-282` (`PlannedRender`) and `:547-719` (`planRender`);
  - `src/lib/strudel/transport.ts:69-101` (`segmentLoop`, `songBarAt`, `locate`) and
    `:242-246` (`clipSegments`); `src/lib/render/timing.ts:14-19` (`layoutSegments`);
  - `crates/strudeland-host/src/planned.rs`.

## What changes (the contract)

Today `strudeland-render` re-derives which section a layout bar plays in
(`plan.rs:92-136`: `section_at`, `section_of_song_bar`, `section_of_clip_bar`) only to name
sections in the unsupported-material refusal. The planner already knows the answer: it lays
the song out with `layoutSegments`, and `locate` on those segments is the transport's own
mapping. After this task:

- `planRender` returns `sections: PlannedSection[]`, where
  `PlannedSection = { fromBar: number; toBar: number; name: string }`. The runs cover the
  layout bars `[0, c1)`, with `c1 = window.toBar + (chunk?.overlapBars ?? 0)` (the `c1` at
  `job-plan.ts:675`, the end of the schedule query). Each run is a maximal stretch of
  consecutive bars where `locate(bar, structure, segments)` gives the same section index.
  `structure` is the plan's (`loopsApplied`) structure and `segments` is the same value the
  query used (`job-plan.ts:563`). A run's `name` is that section's name. Bars where `locate`
  is null (no sections, past the song) are in no run. The runs are in bar order, do not
  overlap, and start at bar 0, so they also cover the lookback bars before `window.fromBar` (the
  `--section` case) and the repeat pass of a loop preset (it plays bars from the same layout,
  below `c1`). They never cover a negative bar, so a negative bar keeps today's `None`
  (reverify §3: unreachable anyway).
- The field is in every plan, single-chunk plans included. It is not in the
  `strudeland-render --info` document (`bin/strudeland-render.rs:179-190` lists its fields;
  unchanged) and not in `PageJobInfo`.
- The Rust mirror (`planned.rs`) gets `PlannedSection` and a **required**
  `sections: Vec<PlannedSection>` (no `#[serde(default)]`). Here is why:
  - The bundle is compiled into the host (`bundle.rs:4`, `include_str!("../js/host.js")`),
    so no host ever reads a plan from an older bundle. The "a newer bundle field does not break
    an older host" rule in the `planned.rs` module doc is about extra fields, and it stays.
  - A required field makes a stale bundle fail loudly with `malformed plan: missing field
    sections`. With `#[serde(default)]` it would silently name no section, and every
    sectioned refusal text would change. That is the one failure this task must never
    allow.
  - It follows the convention the struct already has: `musicFrames` and `contentFrames` are
    required and pinned by `a_wrong_shape_is_rejected`.
  - The host binary re-serializes the plan through this struct (`bin/strudeland-host.rs:173-174`).
    With a required field, `plan-render` always prints `sections`, as the page does, so
    `host:parity --batch` compares the field on both sides. With
    `skip_serializing_if = "Vec::is_empty"`, the empty case would differ.
- `refuse` (`plan.rs:64`) takes `&[PlannedSection]` in place of the window and structure. It
  dedupes, looks each first bar up in the runs (a binary search), and sorts. The three
  mapping functions are deleted, along with their tests (`plan.rs:190-279`). The mapping cases
  move to TS tests of `planRender`'s `sections`.
- **No visible change.** Every refusal text, exit code, engine job, golden, ref audio, render
  vector, `--info` document, CLI output and `SKILL.md` line stays byte-identical for every
  input.

## Stages

There are two stages. They run in order, and each one is one commit that is green on every gate
below.

1. [001-planner-sections.md](001-planner-sections.md): `planRender` computes `sections`, and
   the TS tests move the mapping cases. Then the planner obligations: `host:build`,
   `engine:refs --write` (only provenance moves) and the three `host:parity` modes. The Rust
   mirror gets the required field, and `strudeland-render` still ignores it. This stage also
   records the baseline and runs the equivalence probe, the old Rust mapping against the new
   runs on real host plans.
2. [002-refuse-by-runs.md](002-refuse-by-runs.md): `refuse` names sections from
   `planned.sections`, and the three mapping functions and their tests are deleted. This stage
   is Rust only and touches no planner file, so it regenerates nothing.

Why two stages and not one:

- 001 carries every generated-file obligation (`host.js`, the refs' provenance, parity), and
  its Rust behaviour does not change. Its proof is that nothing moved except the new field,
  and that the field agrees with the code it will replace on every bar of real plans. The
  probe can run only while both exist.
- 002 is the behaviour switch. Its proof is the refusal A/B against the baseline, on a tree
  where the planner is already frozen. The A/B also passes on 001, so a refusal difference in
  002 can come only from 002's own small diff.
- Folding them into one stage would put both proofs into one commit. That loses the
  bisection point and does not save a gate, because 002's gates are cheap.

The stages are sequential: 002 reads the field 001 adds.

## Rules for every stage

- **No visible change** (see the contract). Byte-identical:
  - every refusal text and exit code from `strudeland render` and `strudeland-render`
    (refusal A/B);
  - every engine job text, and every plan field except the new `sections` (plan A/B,
    `host:parity`);
  - `tests/render/goldens/`, `crates/strudeland-render/tests/vectors/`,
    `crates/strudeland-engine/tests/vectors/`;
  - every file in `crates/strudeland-engine/tests/refs/` except the allowed provenance
    changes ([001](001-planner-sections.md), refs A/B);
  - `crates/strudeland-cli/` (its `skill/SKILL.md` included), `src/lib/ipc/`,
    `src/lib/control/`, `crates/strudeland-ctl/`.
- **Text-pinning tests are frozen.** Each of these keeps every expected literal byte for byte
  and passes unedited:
  - `crates/strudeland-render/src/error.rs` tests;
  - `crates/strudeland-render/tests/pipeline.rs` (`samples_are_refused_and_nothing_is_rendered`,
    `an_unsupported_hap_in_the_lookback_refuses_a_section`);
  - `crates/strudeland-render/tests/bin.rs`, `tests/never_panics.rs`;
  - `crates/strudeland-cli/src/render.rs` tests, `crates/strudeland-cli/tests/e2e.rs`,
    `crates/strudeland-cli/src/skill.rs`.

  There is one exception, and it is not a text change. The `plan.rs` refusal tests
  (`nothing_unsupported_is_not_refused`,
  `signatures_dedupe_by_sound_and_keys_with_the_minimum_bar`,
  `the_clip_layout_names_the_clip_section`, `the_linear_layout_names_the_song_section`) must
  change their *inputs* in 002, because `refuse` takes runs instead of a window and structure.
  Their expected strings, sounds, bars and `section` assertions stay byte-identical.
- **Replace tests, don't layer them.** The mapping tests leave Rust in 002 because their
  subject leaves Rust. Their cases are already in TS from 001.
- **New logic ships with tests** in the same commit.
- **Generated files are written only by their commands:**
  - `crates/strudeland-host/js/host.js` and `host.inputs.json`: `bun run host:build`;
  - `crates/strudeland-engine/tests/refs/`: `bun run engine:refs --write`.

  `src/lib/ipc/bindings.ts` is untouched.
- **Don't touch** `src/lib/strudel/transport.ts` or `src/lib/render/timing.ts`. The planner
  calls `locate` and `layoutSegments` as they are.
- **Run `bun run format` and `cargo fmt --all`** before each commit. `bun run format` may
  rewrite files: stage them, then run `bun run lint`.
- **Test cadence.** While working, run only the targeted tests the stage file names. When the
  stage is done, run the full gate once, in order. Log each command and read the log rather than
  re-running it.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan. A later session sets `S` to its own
scratchpad. If `$S/0129-baseline/` is missing there, it rebuilds the baseline as
[001](001-planner-sections.md) step 1 says, from a detached checkout of `ee91dbd` in this
worktree (`git switch --detach ee91dbd`, build, `git switch -` back). Never use a second
worktree. `N` is the stage number (`001` or `002`). `L` logs a command with its start time
and exit code:

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
B="$S/0129-baseline"
L() { log="$S/0129-$N-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

| Step | Command | Stages | Pass |
|---|---|---|---|
| 1 | `L host-build bun run host:build` | 001, 002 | exit 0. In 002, `git diff --exit-code -- crates/strudeland-host/js` (log it as `host-unchanged`) exits 0 too |
| 2 | `L host-diff "$B/host-diff.sh"` | 001 | exit 0: every `host.js` hunk is inside the `// src/lib/render/job-plan.ts` region, and `host.inputs.json` changes only the `job-plan.ts` hash |
| 3 | `L refs-check bun run engine:refs` | 001, 002 | 001: exit 1, and its summary names only provenance and the two floor cases (`mod-lfo-note`, `vowel-formants`). 002: exit 0 |
| 4 | `L refs-write bun run engine:refs --write` | 001 | exit 0 |
| 5 | `L refs-recheck bun run engine:refs` then `L refs-ab "$B/refs-ab.sh"` | 001 | both exit 0 |
| 6 | `L engine-vectors bun run engine:vectors` and `L render-vectors bun run render:vectors` (check mode) | 001, 002 | exit 0 |
| 7 | `L frozen git diff --exit-code ee91dbd -- tests/render/goldens crates/strudeland-render/tests/vectors crates/strudeland-engine/tests/vectors crates/strudeland-cli crates/strudeland-ctl src/lib/ipc src/lib/control src/lib/strudel/transport.ts src/lib/render/timing.ts` | 001, 002 | exit 0 |
| 8 | `bun run format`, then `L bun-test bun test`, `L check bun run check`, `L lint bun run lint` | 001, 002 | exit 0 |
| 9 | `L build cargo build --locked -p strudeland-cli -p strudeland-render -p strudeland-host --bins` | 001, 002 | exit 0 (the A/B scripts use these debug binaries) |
| 10 | `L plan-ab "$B/plan-ab.sh" check "$S/0129-$N-plan"` | 001, 002 | exit 0, ends `equal: <n> cases`, with the baseline's case count |
| 11 | `L refuse-ab "$B/refuse-ab.sh" check "$S/0129-$N-refuse"` | 001, 002 | exit 0, ends `equal: <n> cases`, with the baseline's case count |
| 12 | `L probe cargo test -p strudeland-render --lib sections_probe -- --nocapture` | 001 only (step 9 of the stage file) | exit 0 |
| 13 | `L host-tests cargo test --locked -p strudeland-host` | 001, 002 | exit 0 |
| 14 | `L cargo-test cargo test --locked --workspace` | 001, 002 | exit 0 |
| 15 | `L clippy cargo clippy --locked --workspace --all-targets -- -D warnings` and `L fmt cargo fmt --all --check` | 001, 002 | exit 0 |
| 16 | `L render-build bun run render:build`, then `L parity bun run host:parity`, `L parity-batch bun run host:parity --batch`, `L parity-mm bun run host:parity --mixed-meter` | 001 | exit 0 each |
| 17 | `L perf cargo test --release -p strudeland-host --test perf_budget -- --ignored --nocapture` | 001 | exit 0. Record the median |

Notes:

- Steps 3-5 are the provenance rule (`AGENTS.md`, Rust engine): `job-plan.ts` is in
  `PLANNER_FILES` (`scripts/engine/provenance.ts:30-39`). The floor cases' job files carry
  the provenance, so their job bytes change, and `settleFloorCase` re-renders them
  (`scripts/engine/refs-lib.ts:777-778`). Their WAVs and `floorDb` may move. Nothing else may.
- Step 12 runs while the probe module exists (stage 001, step 9). It is deleted before the
  commit, and step 7's `git diff` and the stage's own checks prove it is gone.
- `host:parity` needs Chromium for Playwright 1.64.1. It is installed
  (`~/Library/Caches/ms-playwright/chromium-1248`, reverify §4).
- `cargo test --workspace` needs cmake, a C compiler and the V8 download. The first build takes
  minutes.
- The gates run on macOS, where `jsmath`'s exact tests are judged.
- **Each stage report lists:**
  - the exit line of every log;
  - the A/B case counts;
  - the `refs-ab` file counts (JSON files changed only in provenance, WAVs changed);
  - the probe's checked-bar count (001);
  - the `perf_budget` median (001);
  - in 001, the `sections` that the plan A/B printed for `song-linear.js` clip and
    `mixed-meter-loop.js` clip.

## STOP, and report to the user, on any of these

Do not work around them:

- any refusal A/B difference, or a needed change to an expected literal in a text-pinning
  test;
- any plan A/B difference, meaning an engine job text or any plan field other than
  `sections`;
- any `host:parity` difference, in any mode;
- a refs change beyond the provenance rule, or a golden or vector change;
- a `host.js` hunk outside the `job-plan.ts` region;
- an equivalence-probe mismatch on any bar;
- `perf_budget` over its budget;
- a need to touch `transport.ts`, `timing.ts`, a threshold, a golden, `SKILL.md`, a CLI flag
  or an A/B script after its baseline was recorded.

Never re-record a baseline, and never loosen a check, to absorb a difference.

## Commits

- Use plain Conventional Commits, with the message each stage file gives and no task or plan
  identifiers. End each one with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed.
