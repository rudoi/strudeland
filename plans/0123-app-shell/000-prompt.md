# 0123 implementation entry point

- **Branch:** `llm/andrew/strudeland/0123-app-shell`.
- **Plan:** [the proposal](proposal.md). It is the contract: the module, the ports, the
  state, the gate table, every verb, the tests, the rejected options and the decisions.
- **Base:** `f7861f5` (`origin/main`). Diff against it, never against a local `main` that may be
  stale.
- **Evidence:** [reverify.md](reverify.md), with file and line at `f7861f5`. Read its
  "Behaviour that must stay identical" and "Boundaries" sections before you edit anything.
- **Style precedent:** `src/lib/play-arbiter.ts` (store plus verbs, dependencies called through
  their objects, a header comment saying what the module decides) and its plan,
  `plans/0115-play-stop-arbitration/proposal.md`.

## Stages

One stage, one commit:

1. [001-shell.md](proposal.md): the shell module, its ports, the Tauri adapter, the fake, the
   gate table, the modal state, `onQuit`, `canCloseWindow`, `start` with the menu sync and the
   instruments drop, the tests, `App.svelte` moved onto it, and the docs.

Why not two: splitting the listener start and the menu sync from the rest needs interim verbs
(`setReady`, an instruments-drop verb, App passing the ports' functions directly) that the
second stage deletes. See the proposal's "Rejected".

## Rules

- **No visible change.** Menu enablement, shortcut behaviour, Quit and window-close outcomes,
  focus and copy stay identical. If something seems to need a change, stop and report it.
- **Keep every gate row as the proposal's table has it,** including the rows that look
  inconsistent. They are #152's to decide.
- **Boundaries.** No Rust (#135). No edit to `src/lib/app.ts`, `src/lib/play-arbiter.ts`,
  `onGesture`, `prepareNoteAudition`, `resumeAudio` or StatusBar (#136). No component other than
  `App.svelte` changes. `session.ts`, `actions.ts`, `app-quit.ts`, `project/api.ts`,
  `export/api.ts` and `export/controller.ts` are not edited.
- **"Only X calls Tauri" holds.** `shell-ports.ts` imports the four existing adapter functions
  and calls nothing else. `shell.ts` imports Tauri types only through `import type`.
- **Only `shell.start()` calls `session.init`.** `init` registers the window-close hook once,
  so the test harness builds the real session without calling `init` (unlike
  `control-handler.test.ts:64`), or `start()`'s `canCloseWindow` would never reach the hook.
- **Replace tests, don't layer them.** The existing `export-dialog`, `session`,
  `project-actions` and `app` tests stay unchanged; the new behaviour is tested in
  `tests/shell.test.ts`.
- **Host bundle.** No edited file is listed in `crates/strudeland-host/js/host.inputs.json`
  (checked while planning); re-check, and run `bun run host:build` only if one is.
- **Don't run** `ipc:build`, `host:build`, `render:check`, `engine:*` or cargo. Nothing they
  cover changes.
- **Test cadence.** While working, `bun test tests/shell.test.ts` (and
  `tests/ipc-adapters.test.ts`). When the stage is done, run each gate once, logging output and
  the exit code, and read the log rather than re-running. `S` is your session's scratchpad.

  ```sh
  bun run format
  bun test > "$S/0123-001-test.log" 2>&1; echo "exit $?" >> "$S/0123-001-test.log"
  bun run check > "$S/0123-001-check.log" 2>&1; echo "exit $?" >> "$S/0123-001-check.log"
  bun run lint > "$S/0123-001-lint.log" 2>&1; echo "exit $?" >> "$S/0123-001-lint.log"
  ```

- **Commits are plain Conventional Commits** with the message the stage gives, no task or plan
  identifiers, ending with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`.
- **Follow-up already filed:** #152 (window close vs Quit, and the gate differences). Cite it in
  the PR body; file anything new with `keitora task new`.
- **Manual smoke.** The user runs the stage's smoke list on a real build (`bun run tauri dev`)
  before the PR is approved. Put the list in the PR body's test plan.
