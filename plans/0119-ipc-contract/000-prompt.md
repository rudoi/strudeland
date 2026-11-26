# 0119 implementation entry point

- **Branch:** `llm/andrew/strudeland/0119-ipc-contract`.
- **Plan:** [proposal.md](proposal.md). It is the contract: the error
  type, the kind table, the generator's input and output, the behaviour table and the
  decisions.
- **Evidence:** [reverify.md](reverify.md), with file and line at the plan commit.
- **Terms:** `GLOSSARY.md` (App boundary): command error, stale root, IPC bindings.

## Stages

The stages run in order. Each one is one commit, green with every gate below.

1. [001-command-error.md](001-command-error.md): the Rust `CommandError`, the typed stale
   project, the `From<ProjectError>` table, and every `String` command moved to it. On the TS
   side, `invokeCommand`, `toCommandError` and the thin subclasses, keeping every message.
2. [002-bindings.md](002-bindings.md): `scripts/ipc/build.ts` (`bun run ipc:build`), the
   committed `src/lib/ipc/bindings.ts` and its staleness test.
3. [003-adapters.md](003-adapters.md): the adapters and fakes use the generated names, args,
   events and kinds. TS `PROJECT_CHANGED` goes, and AGENTS.md is updated.

## Rules for every stage

- **Visible text is byte-identical.** That covers:
  - the status bar, dialogs and inline form errors;
  - memo notices (`memo-ui.ts`) and the recording controller's errors;
  - the export dialog's failure and written paths;
  - the settings notice;
  - the control socket's wire codes and messages, so the CLI's output and exit codes too.

  A kind may only choose a branch that existed before, on the same message. If a stage seems to
  need a text change, stop and report it.
- **No change to the control protocol, the CLI or `SKILL.md`:** `strudeland-ctl`,
  `src/lib/control/protocol.ts` and `crates/strudeland-cli/`.
- **Don't touch `src/lib/errors.ts`** or any file listed in
  `crates/strudeland-host/js/host.inputs.json`. Don't run `host:build`, `engine:refs`,
  `render:vectors` or goldens.
- **Replace tests, don't layer them.** A test of deleted code moves to the code that replaces it,
  with the same assertions; never keep both. A string-equality test of an error becomes a
  kind-and-message test. New logic ships with tests in the same commit.
- **Test cadence.**
  - While working, run targeted tests, such as `cargo test -p strudeland-app project::` or
    `bun test tests/project-api.test.ts`.
  - When the stage is done, run each gate once, log it, and read the log rather than re-running
    it. `S` is the scratchpad directory:

    ```sh
    bun run format
    cargo fmt --all
    bun test > "$S/0119-<stage>-bun-test.log" 2>&1; echo "exit $?" >> "$S/0119-<stage>-bun-test.log"
    bun run check > "$S/0119-<stage>-check.log" 2>&1; echo "exit $?" >> "$S/0119-<stage>-check.log"
    bun run lint > "$S/0119-<stage>-lint.log" 2>&1; echo "exit $?" >> "$S/0119-<stage>-lint.log"
    cargo test --workspace > "$S/0119-<stage>-cargo-test.log" 2>&1; echo "exit $?" >> "$S/0119-<stage>-cargo-test.log"
    cargo clippy --workspace --all-targets -- -D warnings > "$S/0119-<stage>-clippy.log" 2>&1; echo "exit $?" >> "$S/0119-<stage>-clippy.log"
    cargo fmt --all --check > "$S/0119-<stage>-fmt.log" 2>&1; echo "exit $?" >> "$S/0119-<stage>-fmt.log"
    ```

  - `cargo test --workspace` needs `cmake`, a C compiler and the V8 download the host brings. The
    first build takes minutes.
- **`bun test` must not need Tauri.** Mock `@tauri-apps/api/core` and `/event` as
  `tests/project-api.test.ts:49` does. Nothing that imports Playwright is a `*.test.ts`.
- **Windows clippy (`result_large_err`):** box any large `Err` payload a new `Result` carries.
  The native workflow runs clippy on Windows.
- **Commits are plain Conventional Commits.** Use the message the stage file gives, with no task
  or plan identifiers. End each with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

  Run `bun run format` (frontend) and `cargo fmt --all` (Rust) before committing.
- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside, make
  a WIP commit.
