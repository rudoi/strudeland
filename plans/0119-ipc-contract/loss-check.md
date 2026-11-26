# 0119 loss check (refine pass)

Compares the refined plan with the plan commit (1bde592) and the refine findings. The check is
light: the refine changes one TS signature and one module's home, and adds detail elsewhere.

## The original plan

**Preserved:**

- the three stages, their order, scope, decisions 1-13, the commit messages and the trailer;
- the Rust design: `CommandError`, the flat kind enum, the `From<ProjectError>` table,
  `for_memos`, the export, instrument and piano-roll mappings, and the unchanged
  `app_quit_*` and infallible commands;
- the behaviour table and the byte-identical messages, including the control socket's codes;
- the generator's inputs, outputs, ordering, refusals and the staleness test;
- `instanceof ProjectError` at `session.ts:1165`, `:1225` and `:1438`, and decision 6's
  routing of stale reads;
- "replace, don't layer" and the stage gates.

**Modified:**

- **`invokeCommand`'s signature.** It was `invokeCommand<T, N extends CommandName =
  CommandName>(name, ...args): Promise<T>`, which cannot check args per command: with `T`
  explicit, TS does not infer `N`, which falls back to the union. It is now
  `invokeCommand<N extends CommandName>(name: N, ...args: ArgsOf<N>): Promise<unknown>`, and
  each adapter casts its result (`as Promise<T>`). Stage 001 already returns `unknown` and adds
  the casts, so stage 003 only types `name` and `args`. `ArgsOf` indexes the generated
  `CommandArgs[N]` interface (not a generic `CommandArgs<N>`).
- **Explicit argument keys.** `{ ...version }` becomes three named keys, and recording's
  `defined` moves inside its `call`, so every argument is a fresh literal and excess-checked.
  The original kept both shapes and retyped `defined` as `Partial<A>`, which would not catch a
  misspelled extra key.
- **`ExportError`'s home.** It moves from `export/api.ts` to the Tauri-free
  `src/lib/export/error.ts`, so `export/controller.ts` and `tests/export-dialog.test.ts` load no
  `@tauri-apps` module (Bun's `mock.module` is process-global). A shared
  `tests/helpers/tauri-core-mock.ts` with the full core surface is added for the new adapter
  test.
- **`toCommandError`'s `cause`.** Every branch that builds a new error sets `cause: e`, the
  object branch included; the original said so only for the fallback branch, which would have
  lost `written`. A test pins it.
- **The Windows clippy risk** is now sized (about 32 B and 56 B against a 128 B threshold) and
  dropped; the "box it if it fires" fallback went with it.

**Added:**

- the generator's parsing rules, each tied to a real case in `src-tauri/src` (whole-line
  attribute, doc-comment mentions, lifetimes in generics, one-line lists, no `->`,
  non-`Result` returns, `Result` split at depth 0, duplicate `REQUEST_EVENT` names, the
  indented `u32` const), with parser tests, and the regex and byte-compare precedents;
- stage 003's AGENTS.md updates at `:129`, `:277` and `:302-303`, and an `rg` check;
- explicit `for_memos` at every `ProjectError` `?` in `recording.rs` (nine sites, four more
  than the finding listed: `:692`, `:699`, `:705`, `:992`, `:993`);
- the song-kind rename sites, the `ProjectApi` doc rewording, the stale-read control test's
  construction, and why `PROJECT_ERROR_KINDS` leaves out the non-song kinds;
- the malformed-object case (`"[object Object]"`) stays as pinned.

**Removed:** nothing in scope.
