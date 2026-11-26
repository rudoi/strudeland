# 0121 implementation entry point

- **Branch:** `llm/andrew/strudeland/0121-engine-core`.
- **Plan:** [the proposal](proposal.md). It is the contract: Core's interface, the
  settler seam and its two adapters, the deletions, the bit guard and the decisions.
- **Evidence:** [reverify.md](reverify.md), with file and line at `3cd77ba`. Read its "What must
  not change (sound)" section before you edit anything.
- **Background reading:** `crates/strudeland-engine/README.md`, "Live mode"; `AGENTS.md`, the
  "Rust engine" rule.

## Stages

The stages run in order. Each one is one commit, green with every gate below.

1. [001-baseline.md](proposal.md): the A/B baseline, built **before any edit**, and
   `live_equals_offline` extended to local reverbs.
2. [002-core-lifecycle.md](proposal.md): `stop_voices`, `set_seed`, `create_orbit`,
   `frame`, `mix`, `bus_graph`, `push_error`, `drain_errors` and `finish`. Both drivers move onto
   them, and the live tests that read the touched fields are rewritten.
3. [003-settler.md](proposal.md): `IrTarget`, `IrSettler`, `SyncSettler`, the worker
   adapter, `settle_reverbs`/`land_ir`/`cache_ir`, the offline flag, and the deletions.
4. [004-core-module.md](proposal.md): `render_core.rs` with private fields, `Slot`'s
   accessors, the test readers, the `render.rs` test rewrite, and the README and AGENTS.md.

## Rules for every stage

- **Zero bit change.** Every offline render, live render, stem and error stays as it is. If a
  stage seems to need a different output, stop and report it. Never update a baseline, ref,
  vector or golden to absorb a difference.
- **Keep the two convolver builds apart.** `SyncSettler` builds with `Convolver::new`, through
  `settle_cached`. The worker adapter and its fallback use `build_ir`, so `Convolver::new_live`,
  plus `fresh_live`. Never route one through the other.
- **Keep settle timing and order.** Offline settles after each pause's haps. Live settles once
  per quantum, after `poll_ir` and before `step`. The order is orbits in bank order, then the new
  voices in creation order, then chain order within a voice.
- **Don't reorder `Core::schedule` or `Core::step`.** They move file in stage 004, but not one
  statement of them moves.
- **Don't touch `crates/strudeland-live/`.** After every stage,
  `git diff --stat main -- crates/strudeland-live` is empty.
- **Replace tests, don't layer them.** A test of deleted or moved code moves to its replacement
  with the same assertions. Each stage file lists what is deleted, what moves and what stays.
- **New logic ships with tests** in the same commit. The stage files name them.
- **Don't run `engine:refs`, `engine:vectors`, `render:vectors`, `render:check`, `host:build`,
  `host:parity` or `ipc:build`.** Nothing they cover changes.
- **Test cadence.**
  - While working, run targeted tests: `cargo test -p strudeland-engine --lib live::`,
    `cargo test -p strudeland-engine --lib render::`, and
    `cargo test -p strudeland-engine --test live_equals_offline`.
  - When the stage is done, run each gate once, log it, and read the log rather than re-running
    it. `S` is the scratchpad,
    `/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`,
    and `N` is the stage number. That path belongs to the session that wrote this plan. A later
    session has its own scratchpad: set `S` to it, and restore `$S/0121-baseline/` from
    `~/.cache/strudeland-0121/` as [001-baseline.md](proposal.md)
    describes.

    ```sh
    cargo fmt --all
    cargo test -p strudeland-engine -p strudeland-live > "$S/0121-$N-crates.log" 2>&1; echo "exit $?" >> "$S/0121-$N-crates.log"
    cargo test --workspace > "$S/0121-$N-cargo-test.log" 2>&1; echo "exit $?" >> "$S/0121-$N-cargo-test.log"
    cargo clippy --workspace --all-targets -- -D warnings > "$S/0121-$N-clippy.log" 2>&1; echo "exit $?" >> "$S/0121-$N-clippy.log"
    cargo fmt --all --check > "$S/0121-$N-fmt.log" 2>&1; echo "exit $?" >> "$S/0121-$N-fmt.log"
    "$S/0121-baseline/ab.sh" check > "$S/0121-$N-ab.log" 2>&1; echo "exit $?" >> "$S/0121-$N-ab.log"
    ```

  - **The A/B log must end `exit 0` and report zero differences.** That is the absolute bit
    guard. `live_equals_offline` alone is relative.
  - The real-time checks need no extra run: `cargo test --workspace` runs
    `strudeland-live`'s `rt_alloc.rs` and `player.rs`. The dropout and decode benches (`--ignored`)
    measure timing. This refactor moves no work onto the callback, so they are not gates.
  - `cargo test --workspace` needs `cmake`, a C compiler and the V8 download the host brings. The
    first build takes minutes.
  - The gates run on macOS. Clippy there cannot see a `cfg(windows)` or `cfg(unix)` difference.
    The engine has none in the files touched, so this needs no extra care.
- **The A/B guard must exist before any engine edit.** If `$S/0121-baseline/base/` is missing
  (a new session, or a cleared scratchpad), recover it first, as 001 step 6 says. Never record a
  baseline from an edited tree: `ab.sh` refuses to.
- **Commits are plain Conventional Commits.** Use the message the stage file gives, with no task
  or plan identifiers. End each with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside, make
  a WIP commit. Nothing under `$S` is committed.
