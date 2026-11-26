# 0122 implementation entry point

- **Branch:** `llm/andrew/strudeland/0122-engine-capability`.
- **Plan:** [the proposal](../0122-engine-capability.md). It is the contract. It covers:
  - the three modules and what goes in each;
  - the public API table;
  - the derivations;
  - the tests that move, are replaced and are added;
  - the equality guard, the rejected options and the decisions.
- **Base:** `385724a` (`origin/main`). Every diff and check names the hash, because the local
  `main` can be stale.
- **Evidence:** [reverify.md](reverify.md), with file and line at `385724a`. Read its "What must
  not change" section before you edit anything.
- **Background reading:**
  - `crates/strudeland-engine/README.md`, the opening section and "Controls";
  - `crates/strudeland-engine/controls.md`, the header;
  - the "Rust engine" rule in `AGENTS.md`.

## Stages

The stages run in order. Each one is one commit, green with every gate below.

1. [001-split.md](001-split.md) has two parts:
   - the equality guard, set up **before any edit**;
   - `controls.rs` split into `capability.rs`, `params.rs` and `coerce.rs`, with every import on
     the real path and the stale doc paths fixed (AGENTS.md's nonexistent `signatures` module
     among them). Not one line of logic changes.
2. [002-derive.md](002-derive.md):
   - `SourceKind::NAMES`, `DEFAULT_SOUND`, `allowed_sounds()` and `modulatable_targets()`;
   - `ALLOWED_SOUNDS`, `MODULATOR_TARGETS` and `OSC_SOUNDS` deleted;
   - the two-way chain-read test, the target pin and the sound render test;
   - the docs about the derived lists.

## Rules for every stage

- **No output change.** These stay byte-identical:
  - what the engine accepts or refuses;
  - every `signatures` output and the `controls --json` dump;
  - every render, refusal text, exit code and live warning.

  If a stage seems to need a different output, stop and report it. Never re-record the baseline,
  and never edit a ref, vector, golden or the shared signature table to absorb a difference.
- **Keep `{"s":["-"]}` accepted.** The sound check keeps its `SILENCE` half.
- **Keep the hand lists that are records.** `ALLOWED_KEYS` stays hand-kept, checked against
  `controls.md`. So does `CHAIN_KEYS`, checked against `parse_entry_params`. Never edit either one
  to make a test pass. A mismatch is a finding.
- **No TypeScript edits.** `src/lib/render/schedule.ts:166` keeps its stale "controls.rs" comment
  (it is a planner file; see the proposal's Scope). After every stage,
  `git diff --stat 385724a -- src scripts tests vendor crates/strudeland-host crates/strudeland-live`
  is empty.
- **Diff against `385724a`, never `main`.** `385724a` is `origin/main`. The local `main` branch
  can be stale (it was `77e210f` while planning), and a diff against it shows unrelated
  changes.
- **Replace tests, don't layer them.** A test of moved code moves with it, with the same
  assertions. Each stage file lists what moves, what is replaced and what is added.
- **New logic ships with tests** in the same commit. Stage 002 names them.
- **Don't run `engine:refs`, `engine:vectors`, `render:vectors`, `render:check`, `host:build`,
  `host:parity`, `ipc:build` or `bun test`.** Nothing they cover changes.
- **Test cadence.**
  - **While working,** run targeted tests:
    - `cargo test -p strudeland-engine --lib capability::`, and the same for `params::`,
      `coerce::`, `modulate::` and `chain::`;
    - `cargo test -p strudeland-engine --test bin`;
    - `cargo test -p strudeland-engine --test never_panics`.
  - **When the stage is done,** run each gate once, log it, and read the log rather than
    re-running it.
    - `S` is the scratchpad,
      `/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
      That path belongs to the session that wrote this plan. A later session has its own
      scratchpad: set `S` to it and restore `$S/0122-baseline/` as
      [001-split.md](001-split.md) step 1 describes.
    - `N` is the stage number.

    ```sh
    cargo fmt --all
    cargo test -p strudeland-engine -p strudeland-render > "$S/0122-$N-crates.log" 2>&1; echo "exit $?" >> "$S/0122-$N-crates.log"
    cargo test --workspace > "$S/0122-$N-cargo-test.log" 2>&1; echo "exit $?" >> "$S/0122-$N-cargo-test.log"
    cargo clippy --workspace --all-targets -- -D warnings > "$S/0122-$N-clippy.log" 2>&1; echo "exit $?" >> "$S/0122-$N-clippy.log"
    cargo fmt --all --check > "$S/0122-$N-fmt.log" 2>&1; echo "exit $?" >> "$S/0122-$N-fmt.log"
    "$S/0122-baseline/eq.sh" check > "$S/0122-$N-eq.log" 2>&1; echo "exit $?" >> "$S/0122-$N-eq.log"
    ```

  - **The equality log must end `exit 0` and report `equal`.** That is the absolute guard for
    the dump and for the `signatures` and `render` outputs of its 226 jobs. The unit tests alone
    are relative. The guard runs only the offline binary. So live warnings rest on `live.rs`'s
    unit tests and `tests/live_equals_offline.rs`, which run in `cargo test --workspace`. See
    the proposal's "What must not change" for what each guard proves.
  - `cargo test --workspace` needs `cmake`, a C compiler and the V8 download the host brings. The
    first build takes minutes.
  - The gates run on macOS. The touched files have no `cfg(windows)` or `cfg(unix)` code, so
    clippy on macOS sees everything.
- **The equality guard must exist before any engine edit.** If `$S/0122-baseline/base/` is
  missing (a new session, or a cleared scratchpad), recover it first, as 001 step 1 says. Never
  record a baseline from an edited tree: `eq.sh` refuses to.
- **Commits are plain Conventional Commits.** Use the message the stage file gives, with no task
  or plan identifiers. End each with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside, make
  a WIP commit. Nothing under `$S` is committed.
- **Follow-up to file** (the workflow's follow-up rule): fix the "controls.rs js_to_number"
  comment in `src/lib/render/schedule.ts:166` with the next planner edit (it then names
  `coerce.rs`). Reference the task number in this plan, never in commits or code.
