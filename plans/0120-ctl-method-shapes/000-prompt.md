# 0120 implementation entry point

- **Branch:** `llm/andrew/strudeland/0120-ctl-method-shapes`.
- **Plan:** [the proposal](../0120-ctl-method-shapes.md). It is the contract: the example schema,
  the interfaces (`decode_result`, `codes::ALL`, the limits, `wire_fixtures`, the CLI's
  `call::<M>`), the rules each reader checks, and the decisions.
- **Evidence:** [reverify.md](reverify.md), with file and line at `3a189da`. Its "Why Rust will
  not validate params first" section is why the webview stays the only validator. Read it before
  you write any params example.
- **Terms:** `GLOSSARY.md` (App boundary). Stage 004 adds "Wire example".

## Stages

The stages run in order. Each one is one commit, green with every gate below.

1. [001-ctl-wire.md](001-ctl-wire.md): the ctl constants, `decode_result`, the `Rpc` bounds, the
   18 wire examples, `tests/wire.rs`, the `wire-fixtures` feature, and the pruned `methods.rs`
   tests.
2. [002-ts-readers.md](002-ts-readers.md): `tests/helpers/wire-examples.ts`, the protocol and
   handler tests reading the examples, the deleted regex scrapes, and the `codes` macro.
3. [003-cli-typed.md](003-cli-typed.md): the CLI's typed `call::<M>` and `report::<M>` at all 16
   call sites, `EVENTS_LIMIT_MAX` in `cli.rs`, the help and `SKILL.md` limits test, and the CLI tests
   reading the examples.
4. [004-app-readers.md](004-app-readers.md): `control.rs`'s refusal tests reading the examples,
   and the AGENTS.md and GLOSSARY.md updates.

## Rules for every stage

- **No wire change.** These all stay byte for byte:
  - every request the CLI sends;
  - every params value the webview accepts or refuses, and the order of its checks;
  - every error code and message;
  - every CLI output (stdout and stderr), help text, flag and exit code;
  - every sentence of `SKILL.md`.

  If an example seems to need a different text, the example is wrong: copy the current text from
  the code (`handler.ts`, `control.rs`). If a stage seems to need a behaviour change, stop and
  report it.
- **Don't edit `handler.ts`'s validation.** Stage 002 only adds `export` to `EVENTS_LIMIT` and
  `EVENTS_LIMIT_MAX`.
- **Don't touch `src/lib/ipc/bindings.ts`, `src/lib/errors.ts` or any file listed in
  `crates/strudeland-host/js/host.inputs.json`.** Don't run `ipc:build`, `host:build`,
  `engine:refs`, `render:vectors` or goldens. None of them is affected.
- **Replace tests, don't layer them.** A test of deleted code moves to the code that replaces it
  with the same assertions. Never keep both. Each stage file lists what is deleted, what moves
  and what stays.
- **New logic ships with tests** in the same commit: `same()`, `decode_result`,
  `expectWireShape` and the limits pins.
- **Test cadence.**
  - While working, run targeted tests: `cargo test -p strudeland-ctl`,
    `bun test tests/control-handler.test.ts`, `cargo test -p strudeland-cli run::`.
  - When the stage is done, run each gate once, log it, and read the log rather than re-running
    it. `S` is the scratchpad directory:

    ```sh
    bun run format
    cargo fmt --all
    bun test > "$S/0120-<stage>-bun-test.log" 2>&1; echo "exit $?" >> "$S/0120-<stage>-bun-test.log"
    bun run check > "$S/0120-<stage>-check.log" 2>&1; echo "exit $?" >> "$S/0120-<stage>-check.log"
    bun run lint > "$S/0120-<stage>-lint.log" 2>&1; echo "exit $?" >> "$S/0120-<stage>-lint.log"
    cargo test --workspace > "$S/0120-<stage>-cargo-test.log" 2>&1; echo "exit $?" >> "$S/0120-<stage>-cargo-test.log"
    cargo clippy --workspace --all-targets -- -D warnings > "$S/0120-<stage>-clippy.log" 2>&1; echo "exit $?" >> "$S/0120-<stage>-clippy.log"
    cargo fmt --all --check > "$S/0120-<stage>-fmt.log" 2>&1; echo "exit $?" >> "$S/0120-<stage>-fmt.log"
    ```

  - After the gates, `git diff -- crates/strudeland-ctl/tests/wire/` is empty: the examples are
    committed as `bun run format` leaves them. Prettier keeps `1.0` as written but rewrites
    `1.50` as `1.5`, so read any diff it made there for a changed value before staging it.
  - `cargo test --workspace` needs `cmake`, a C compiler and the V8 download the host brings. The
    first build takes minutes.
  - The gates run on macOS, so clippy there cannot see an import that is unused only on Windows,
    where `#[cfg(unix)]` tests are compiled out (`.github/workflows/native.yml` runs clippy with
    `-D warnings`). Reason about `cfg(unix)` by reading: an import used only by unix-gated tests
    is gated the same way.
- **`bun test` stays browser-free and Tauri-free.** The TS readers use `node:fs` only.
- **The examples are hand-written and prettier-formatted.** Run `bun run format` after editing
  any file under `crates/strudeland-ctl/tests/wire/`. Do not add them to `.prettierignore`.
- **Commits are plain Conventional Commits.** Use the message the stage file gives, with no task
  or plan identifiers. End each with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside, make
  a WIP commit.
