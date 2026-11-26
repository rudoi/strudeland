# 0118 implementation entry point

- **Branch:** `llm/andrew/strudeland/0118-one-finishing-path`.
- **Proposal:** [../0118-one-finishing-path.md](../0118-one-finishing-path.md). It is the
  contract: the interface, the finishing sequence, the byte-identity arguments and the
  decisions.
- **Evidence:** [reverify.md](reverify.md), with file and line at the plan commit.
- **Terms:** `GLOSSARY.md` (Audio export): file cut, loop splice, finishing, leveling strategy.

## Stages

The stages run in order. Each one is one commit, green with `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`.

1. [001-characterization.md](001-characterization.md): the permanent finishing golden, the
   `finish_export` seam and the `testing::audio` fixture.
2. [002-render-planned.md](002-render-planned.md): split planning from rendering, with an
   injected chunk renderer and hand-built plan tests.
3. [003-encode-request.md](003-encode-request.md): the `EncodeRequest` with an explicit MP3
   flush and loop range. `Real` becomes a unit struct, and `loop_points` sits beside the
   encoders.
4. [004-loop-splice.md](004-loop-splice.md): the pure loop splice module, with synthetic-PCM
   tests.
5. [005-finishing-loop.md](005-finishing-loop.md): one finishing loop with the `Leveling`
   switch. This deletes `finish_file`, `encode::encode`, `encode_with` and `finish_loop`.
6. [006-docs.md](006-docs.md): the README, `lib.rs`, and the `AGENTS.md` rule and Layout.

## Rules for every stage

- **Bytes are frozen.** `characterization::finished_output_is_frozen` must pass unchanged at
  the end of every stage after 001. "Unchanged" covers both of these:
  - `tests/golden/finish.json` is never rewritten after 001.
  - `src/characterization.rs` is not edited after 001. A pure import-path fix is the only
    exception, and it must leave the golden byte-identical.

  If a stage seems to need a golden change, stop and report it with the differing cases. Do not
  refresh it. The golden is refreshed only after an encoder crate bump (`mp3lame-encoder`,
  `mp3lame-sys`, `vorbis_rs`, `opus`, `opusic-sys`, `ogg`, `rubato`, `symphonia`), and never in
  this task. Run it on macOS aarch64, the reference platform.
- **Everything else is frozen too:**
  - the public signatures of `render`, `render_audio`, `finish` and `render_export`;
  - `ExportFile`, `ExportRole` and `LoopPoints`;
  - `export::Pipeline`;
  - CLI output and exit codes;
  - app behaviour.

  No `SKILL.md` edit, and no CLI or `src-tauri` edit.
- **No planner-file edits.** Do not touch `src/lib/render/*`, either `timing.ts`, or anything
  listed in `crates/strudeland-host/js/host.inputs.json`. No `host:build`, `engine:refs`,
  `render:vectors` or goldens other than the one stage 001 creates.
- **Replace tests, don't layer them.** A test of deleted code moves to the code that replaces it,
  with the same assertions; never keep both. New logic ships with tests in the same commit.
- **Test cadence.**
  - While working, run targeted tests, such as `cargo test -p strudeland-render
    characterization` or `cargo test -p strudeland-render finish::`.
  - When the stage is done, run each gate once, log it, and read the log rather than re-running
    it. `S` is the scratchpad directory:

    ```sh
    cargo fmt --all
    cargo fmt --all --check > "$S/0118-<stage>-fmt.log" 2>&1; echo "exit $?" >> "$S/0118-<stage>-fmt.log"
    cargo clippy --workspace --all-targets -- -D warnings > "$S/0118-<stage>-clippy.log" 2>&1; echo "exit $?" >> "$S/0118-<stage>-clippy.log"
    cargo test --workspace > "$S/0118-<stage>-test.log" 2>&1; echo "exit $?" >> "$S/0118-<stage>-test.log"
    ```

  - Building the render crate needs `cmake` and a C compiler. `cargo test -p strudeland-cli`
    needs the V8 download the host brings.
- **The crate lint** `deny(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)`
  applies outside `cfg(test)` in `strudeland-render`. That includes `testing.rs`, `splice.rs`
  and `finish.rs`.
- **Windows clippy** (`result_large_err`): box any large `Err` payload a new `Result` carries.
  `export/error.rs:40` is the precedent. The native workflow runs clippy on Windows, where paths
  are wider.
- **Commits are plain Conventional Commits.** Use the message the stage file gives, with no task
  or plan identifiers. End each with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

  Run `cargo fmt --all` before committing. No frontend file changes, so `bun run format` is not
  needed.
- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside, make
  a WIP commit.
