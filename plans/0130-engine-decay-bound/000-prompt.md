# 0130 implementation entry point

- **Branch:** `llm/andrew/strudeland/0130-engine-decay-bound`.
- **Base:** `b7b18ce`. Diff against it, never against a local `main`, which may be stale. The
  branch commits on top of it before stage 001 are plan documents only
  (`git diff --name-only b7b18ce HEAD` lists only `plans/`).
- **Proposal:** [proposal.md](proposal.md). **Evidence:** [reverify.md](reverify.md). Its
  Decisions section (2026-11-26) is binding. Read §2 (what the bound computes), §3 (the engine
  side) and §4 (the djf divergence and the intended truncations) before you edit anything.
- **Background reading:**
  - `AGENTS.md`: the Rules on the Rust engine, the headless host and the render pipeline;
  - `src/lib/render/schedule.ts:178-257` (`FILTER_MAX_TAIL_SEC`, `exportFilterTailSec`) and
    `:335-349` (`exportTimingEvent`);
  - `src/lib/render/lookback.ts:45-50` (`delayTailSec`) and `:142-185` (`exportDecay`);
    `src/lib/render/thresholds.ts:15-18`;
  - `tests/render-job-plan.test.ts:1679-1745`, the TS pins of the same tails, which are the
    model for the parity cases;
  - `crates/strudeland-host/tests/plan_render.rs:16-25,119-167` (a preset request and its
    `export.tail_frames`);
  - `crates/strudeland-engine/src/lib.rs`, `biquad.rs:24-31`, `delay.rs:34-49`,
    `reverb.rs:262-272`, `vowel.rs:20-30`, `djf.rs:40-60`.

## What changes (the contract)

- **New:** `crates/strudeland-render/tests/decay_bound.rs`, with two groups of tests:
  - *parity*: the TS bound's constants, read from host `plan-render` one-shot tails, equal the
    engine's;
  - *completeness*: every family case (33; 44 after the code review), rendered past
    `tail_frames`, stays under its floor.
  - The parity group has 11 cases (12 after the code review, with the compressor).
- **Engine, visibility and naming only:**
  - `pub mod decay` in `strudeland-engine/src/lib.rs` re-exports `biquad::{MAX_TAIL_TIME,
    MAX_TAIL_AMPLITUDE}`, `delay::{MAX_FEEDBACK, MAX_DELAY_TIME, FEEDBACK_EXTRA_FRAMES}`,
    `reverb::IR_DURATION_FACTOR` and `vowel::FORMANTS`;
  - `FEEDBACK_EXTRA_FRAMES` loses `#[cfg(test)]`;
  - `reverb.rs` names its `1.5` as `IR_DURATION_FACTOR` at the one non-test use (`:269`); the
    doc and test literals stay as they are;
  - the `decay` module's doc comment names no Blink source file (no `*.cc`/`*.h` word and no
    `third_party/blink`), or `tests/licence.rs` would demand the Blink header on `lib.rs`.

  No rendered sample changes.
- **Docs:**
  - `crates/strudeland-render/README.md` gains a subsection describing the post-roll bound: what
    it budgets, the delay echoes cut at −60 dB (intended, decision 3) with the measured margins,
    the filter ring budgeted below 24-bit resolution, the DJ filter's flat 0.5 s as a known
    difference (the measured leftover and need, the source level, and that the leftover grows
    with it), and the test that pins it;
  - `AGENTS.md` gets one sentence in the render pipeline rule.
- **No visible change.** Every engine job, plan, export file, one-shot length, golden, ref,
  vector, `host.js`, CLI output and `SKILL.md` line is byte-identical.

## Stages

There is one stage: [001-decay-bound-checks.md](001-decay-bound-checks.md), one commit.

## Rules

- **Frozen files.** `git diff b7b18ce` must be empty for:
  - every `PLANNER_FILES` entry (`scripts/engine/provenance.ts:30-39`) and
    `src/lib/render/thresholds.ts`;
  - `tests/render/goldens/`;
  - `crates/strudeland-engine/tests/refs/` and `crates/strudeland-engine/tests/vectors/`;
  - `crates/strudeland-render/tests/vectors/` and `crates/strudeland-render/tests/golden/`;
  - `crates/strudeland-host/js/`;
  - `crates/strudeland-cli/`.
- **Allowed files.** Outside `plans/`, the diff touches only:
  - `crates/strudeland-engine/src/{lib,delay,reverb,compressor}.rs` (`compressor.rs`: `K_PRE_DELAY`
    made `pub`, visibility only, added in the code review);
  - `crates/strudeland-render/tests/decay_bound.rs`;
  - `crates/strudeland-render/README.md`;
  - `AGENTS.md`.

  Anything else is a STOP.
- **The engine edits are behaviour-free.** Only `pub`, the removed `#[cfg(test)]`, the
  re-export module, and the one named constant. No other line of `crates/strudeland-engine/src`
  changes.
- **The floors are fixed.**
  - The general floor is `decay::MAX_TAIL_AMPLITUDE`, the constant itself (1/32768 ≈ −90.3 dBFS,
    at least as strict as decision 4's −90 dBFS).
  - Delay cases use `1e-3 ×` the render's peak before the bound (−60 dB, decision 3).
  - DJ filter cases use their own floor, −70 dBFS (the user's re-decision, reverify Decisions:
    about 6 dB above the measured −76.9 dBFS).
  - The parity tolerance is ±1 frame.
  - The window after the bound is `EXTRA = 5 s`.

  Never raise a floor, widen the tolerance, shorten `EXTRA`, or drop or soften a case to make a
  test pass.
- **New logic ships with tests.** Here the new logic is the tests. Each assertion message names
  its case.
- **Run `cargo fmt --all`** before the commit. No frontend file changes, so `bun run format` is
  not needed. Run `bun run lint` anyway as a gate.
- **Test cadence.** While working, run only `cargo test -p strudeland-render --test
  decay_bound`. When the stage is done, run the full gate below once, in order. Log each command
  and read the log rather than re-running it.
- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan. A later session sets `S` to its own
scratchpad. `L` logs a command with its start time and exit code:

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
N=001
L() { log="$S/0130-$N-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

| Step | Command | Pass |
|---|---|---|
| 1 | `L decay cargo test --locked -p strudeland-render --test decay_bound -- --nocapture --test-threads=1` | exit 0. The log holds the measured table |
| 2 | `L frozen git diff --exit-code b7b18ce -- src/lib/render/plan.ts src/lib/render/engine-job.ts src/lib/render/lookback.ts src/lib/render/schedule.ts src/lib/render/job-plan.ts src/lib/render/page-job.ts src/lib/render/timing.ts src/lib/strudel/timing.ts src/lib/render/thresholds.ts tests/render/goldens crates/strudeland-engine/tests/refs crates/strudeland-engine/tests/vectors crates/strudeland-render/tests/vectors crates/strudeland-render/tests/golden crates/strudeland-host/js crates/strudeland-cli` | exit 0 |
| 3 | `L files "$S/0130-files.sh"` (below) | exit 0 (no file outside the allowed list) |
| 4 | `L engine-src git diff b7b18ce -- crates/strudeland-engine/src` | exit 0. Read the log: every hunk is a visibility change, the `decay` module, or the `IR_DURATION_FACTOR` naming |
| 5 | `L engine-tests cargo test --locked -p strudeland-engine` | exit 0 (micro, fixtures, v8_math and licence against the unchanged refs) |
| 6 | `L engine-vectors bun run engine:vectors` and `L render-vectors bun run render:vectors` (check mode) | exit 0 each |
| 7 | `L bun-test bun test` and `L lint bun run lint` | exit 0 each (`bun test` includes the `host.js` staleness test) |
| 8 | `L cargo-test cargo test --locked --workspace -- --skip dropout_short` | exit 0 |
| 9 | `L clippy cargo clippy --locked --workspace --all-targets -- -D warnings` and `L fmt cargo fmt --all --check` | exit 0 each |

Step 3's script, written once to `$S/0130-files.sh` (`chmod +x`). `grep` exits 1 when every
changed path is on the allowed list:

```sh
#!/bin/sh
cd /Volumes/rudenet/code/strudeland-worktrees/poptart || exit 2
git diff --name-only b7b18ce -- . ':(exclude)plans' |
  grep -vxE 'crates/strudeland-engine/src/(lib|delay|reverb|compressor)\.rs|crates/strudeland-render/tests/decay_bound\.rs|crates/strudeland-render/README\.md|AGENTS\.md'
test $? -eq 1
```

Not needed, because no planner file, bundle input or Chromium-side input changes: `host:build`,
`engine:refs`, `host:parity`, `render:check`.

**The stage report lists:**

- the exit line of every log;
- the measured table from step 1 (every case: `tail_frames`, peak after the bound in dBFS, the
  floor);
- each parity case's `ΔF` against its expected value;
- the djf numbers as written into the README;
- each delay case's margin under its −60 dB floor (the prototype measured 2.4 dB at feedback
  0.98/0.99, time 0.1; 1.5 dB at time 1.5, feedback 0.9; 8.3 dB at 0.5/0.5).

## STOP, and report to the user, on any of these

Do not work around them:

- **a completeness failure:** any case whose peak after the bound is above its floor, the djf
  cases included: a djf case above −70 dBFS still STOPs, and the user decides the djf allowance
  again;
- **a parity mismatch:** a TS value that differs from the engine constant beyond ±1 frame, or a
  vowel formant whose `6·Q/f` exceeds the 1 s allowance;
- **a case the host or engine refuses** (plan error, unsupported signature, render error), or a
  family that cannot be expressed as a song. Never drop the case;
- **an extended render whose first `frames` differ** from the planned render;
- **any change to a frozen file:** a planner file, golden, ref, vector, `host.js`, the finish
  golden or the CLI;
- **any engine source edit** beyond the visibility and naming the contract allows;
- **any need to loosen a floor, the tolerance or `EXTRA`.**

## Commits

- Use plain Conventional Commits, with the message the stage file gives and no task or plan
  identifiers. End it with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```
