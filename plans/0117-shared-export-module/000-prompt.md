# 0117 implementation entry point

- **Branch:** `llm/andrew/strudeland/0117-shared-export-module`.
- **Proposal:** [../0117-shared-export-module.md](../0117-shared-export-module.md). It is the
  contract: interface, order, wording tables and publish rule.
- **Evidence:** [reverify.md](reverify.md).
- **What changed in the refine:** [loss-check.md](loss-check.md).

## Stages

The stages run in order. Each one is one commit, green with `cargo test --workspace` and
`cargo clippy --workspace --all-targets -- -D warnings`.

1. [001-module.md](001-module.md): rename `export.rs` to `loop_export.rs` and add `export/` with
   the `Pipeline` seam, `RealPipeline`, `FakePipeline` and the table and publish tests. Every old
   public item stays, and no caller changes.
2. [002-cli.md](002-cli.md): `strudeland render` moves onto the module. `RenderBackend` and
   `StrudelandBackend` are deleted, and the CLI tests move onto `FakePipeline`.
3. [003-app.md](003-app.md): the desktop export moves onto the module. The parity test is
   deleted.
4. [004-privatise.md](004-privatise.md): privatise, delete the old resolvers, move
   `tests/stems.rs`, and update `GLOSSARY.md` and the `AGENTS.md` Layout.

## Rules for every stage

- **Behaviour is frozen.** The following must not change:
  - CLI stdout, stderr, flags and exit codes;
  - app error text, progress events and results;
  - file names and paths;
  - overwrite semantics;
  - rendered bytes.

  If a step seems to need a visible change, stop and report it. Do not make it.
- **Replace tests, don't layer them.** A test of deleted code moves to the new code or is
  replaced by a module test that covers the same assertion. Never keep both. New logic ships with
  tests in the same commit.
- **No planner-file edits.** Do not touch `src/lib/render/plan.ts`, `engine-job.ts`,
  `lookback.ts`, `schedule.ts`, `job-plan.ts` or `page-job.ts`, either `timing.ts`, or anything
  listed in `crates/strudeland-host/js/host.inputs.json`. No `host:build`, `engine:refs` or
  goldens.
- **No `SKILL.md` edit.** The CLI surface does not change.
- **Test cadence.**
  - While working, run targeted tests such as `cargo test -p strudeland-render export::` or
    `cargo test -p strudeland-cli render::`.
  - When the stage is done, run each gate once, log it, and read the log rather than re-running
    it. `S` is the scratchpad directory:

    ```sh
    cargo fmt --all
    cargo fmt --all --check > "$S/0117-<stage>-fmt.log" 2>&1; echo "exit $?" >> "$S/0117-<stage>-fmt.log"
    cargo clippy --workspace --all-targets -- -D warnings > "$S/0117-<stage>-clippy.log" 2>&1; echo "exit $?" >> "$S/0117-<stage>-clippy.log"
    cargo test --workspace > "$S/0117-<stage>-test.log" 2>&1; echo "exit $?" >> "$S/0117-<stage>-test.log"
    ```

  - `cargo test -p strudeland-cli` needs the V8 download the host brings. Building the render
    crate needs `cmake` and a C compiler.
- **Commits are plain Conventional Commits.** Use the message the stage file gives. No task or
  plan identifiers. End each with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

  Run `cargo fmt --all` before committing. No frontend files change, so `bun run format` is not
  needed.
- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside, make
  a WIP commit.
- **The crate lint** `deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)`
  applies outside `cfg(test)` in `strudeland-render`. That includes `export/fake.rs`, which is
  compiled into non-test builds.
