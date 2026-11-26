# 0127 implementation entry point

- **Branch:** `llm/andrew/strudeland/0127-fx-entry-walk`.
- **Plan:** [the proposal](../0127-fx-entry-walk.md). It is the contract. It covers:
  - `StageSpec`, the walk and `build_stage`;
  - `FmStep` and `NodeName`;
  - the A/B guard;
  - the tests that move;
  - the rejected options and the decisions.
- **Base:** `416caa6` (`origin/main`). Every diff and check names the hash, because the local
  `main` is stale. Never diff against `main`.
- **Evidence:** [reverify.md](reverify.md), with file and line at `416caa6`. Read its "Orders
  that are visible" section before you edit anything.
- **Background reading:**
  - `crates/strudeland-engine/README.md`, "Effects, worklets, modulation and routing";
  - the "Rust engine" rule in `AGENTS.md`;
  - `plans/0121-engine-core/proposal.md`, "The bit guard", and `plans/0122-engine-capability/proposal.md`,
    "The equality guard". The guard here merges the two.

## Stages

The stages run in order. Each later stage builds on the one before. Run each stage's gates
before starting the next.

1. [001-baseline.md](001-baseline.md): the A/B guard, built and recorded twice **before any
   engine edit**, then snapshotted. Commits nothing.
2. [002-stage-walk.md](002-stage-walk.md): `StageSpec` and `VoiceParams::entry`; `NodeTable`
   stores specs; `Voice::build` plans first and walks; `build_stage`. Deleted: `build_entry`,
   `push_stage`, `push_shaper`, the debug assertion and `the_plan_names_the_built_stages`
   (`StageKind::of` becomes test-only). The walk's error-order test. Also the README and
   AGENTS.md.
3. [003-fm-steps.md](003-fm-steps.md): `FmStep`, and `apply_fm` built from the planned walk.
4. [004-node-names.md](004-node-names.md): `NodeName`, and the `TARGETS` cross-check.
5. [005-tests.md](005-tests.md): the structural tests on the pure plan. Deleted: `kind()` and its
   helpers. The voice-level throw-order test, and the README re-read.

## Rules for every stage

- **Zero output change.** These stay byte-identical:
  - every offline render, live render and stem;
  - every error text, the throw order and the "Available targets" log order;
  - the `controls --json` dump, every `signatures` output and the modulator targets.

  Live timing must not change either: no work moves onto the output callback, and
  `crates/strudeland-live/` is not edited. If a stage seems to need a different output, stop
  and report it. Never re-record the baseline, and never edit a ref, vector, golden,
  threshold or the shared signature table to absorb a difference.
- **Keep the visible orders.**
  - The fallible calls run in today's order, per stage and between stages (the table in the
    proposal's `build_stage`).
  - The registration order is today's, `vib` after the FM for the worklet synths included.
  - `StageId` equals the stage's index in `Voice::stages`.
- **One place decides.** After stage 002, no code outside `NodeTable::plan`, `plan_entry` and
  `plan_fm` tests whether a stage exists. The exceptions are the vibrato and the pulse-width
  LFO (proposal, Decision 4) and the sends (out of scope).
- **Replace tests, don't layer them.** A test of deleted or moved code moves to its replacement
  with the same assertions. Each stage file lists what is deleted, what moves and what is added.
  An assertion whose expected value has to change is a STOP, not an edit.
- **New logic ships with tests** in the same commit. The stage files name them.
- **Don't run** `engine:refs`, `engine:vectors`, `render:vectors`, `render:check`,
  `host:build`, `host:parity`, `ipc:build` or `bun test`. No TS file, planner file or
  `host.inputs.json` input is touched. After every stage,
  `git diff --stat 416caa6 -- src scripts tests vendor crates/strudeland-host crates/strudeland-live crates/strudeland-engine/tests/refs crates/strudeland-engine/tests/vectors`
  is empty.
- **Test cadence.**
  - **While working,** run targeted tests:
    - `cargo test -p strudeland-engine --lib chain::`, and the same for `voice::`,
      `modulate::` and `params::`;
    - `cargo test -p strudeland-engine --test never_panics`;
    - `cargo test -p strudeland-engine --test live_equals_offline`.
  - **When the stage is done,** run each gate once, log it, and read the log rather than
    re-running it.
    - `S` is your session's scratchpad. The planning session's was
      `/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
      A later session sets `S` to its own and restores the guard as
      [001-baseline.md](001-baseline.md) "Recovery" says.
    - `N` is the stage number (`002` … `005`).

    ```sh
    cargo fmt --all
    cargo test -p strudeland-engine > "$S/0127-$N-engine.log" 2>&1; echo "exit $?" >> "$S/0127-$N-engine.log"
    cargo test --workspace > "$S/0127-$N-workspace.log" 2>&1; echo "exit $?" >> "$S/0127-$N-workspace.log"
    cargo clippy --workspace --all-targets -- -D warnings > "$S/0127-$N-clippy.log" 2>&1; echo "exit $?" >> "$S/0127-$N-clippy.log"
    cargo clippy -p strudeland-engine --all-targets --target x86_64-pc-windows-msvc -- -D warnings > "$S/0127-$N-clippy-win.log" 2>&1; echo "exit $?" >> "$S/0127-$N-clippy-win.log"
    cargo fmt --all --check > "$S/0127-$N-fmt.log" 2>&1; echo "exit $?" >> "$S/0127-$N-fmt.log"
    sh "$S/0127-baseline/ab.sh" check > "$S/0127-$N-ab.log" 2>&1; echo "exit $?" >> "$S/0127-$N-ab.log"
    ```

  - **Every log must end `exit 0`.** The A/B log must start its last report line with `0
    differences`. That is the absolute guard. The unit tests alone are relative: they move with
    the code.
  - **The Windows clippy run.** It needs the `x86_64-pc-windows-msvc` target. It was installed
    on the planning machine (`rustup target list --installed`). If it is missing, or the run
    fails for a toolchain reason (a build script or a missing linker, not a lint), record that
    in the stage report and go on. A lint is a failure. The touched files have no `cfg(windows)`
    code, so this run is a second opinion, not the only one.
  - `cargo test --workspace` needs `cmake`, a C compiler and the V8 download the host brings. It
    runs `strudeland-live`'s `rt_alloc.rs` and `player.rs`. The dropout and decode benches
    (`--ignored`) measure timing. They are not gates, because no work moves onto the callback.
- **The guard must exist before any engine edit.** If `$S/0127-baseline/base/` is missing,
  recover it first ([001-baseline.md](001-baseline.md) "Recovery"). Never record a baseline
  from an edited tree: `ab.sh baseline` refuses to.
- **STOP and report** (do not commit, do not "fix" the expectation) when:
  - the A/B check reports any difference;
  - an error text, the throw order or a log order changes, in the guard or in a unit test;
  - a threshold, ref, vector, golden or the signature table seems to need an edit;
  - a test's expected value has to change, rather than its access path;
  - a panic appears in `never_panics`, the digest or a test;
  - the guard cannot be restored or rebuilt.
- **Commits are plain Conventional Commits.** Use the message the stage file gives, with no task
  or plan identifiers in the message or in code comments. End each with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside, make
  a WIP commit. Nothing under `$S` or `~/.cache/strudeland-0127/` is committed.
