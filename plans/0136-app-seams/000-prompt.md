# 0136 implementation entry point

- **Branch:** `llm/andrew/strudeland/0136-app-seams`.
- **Base:** `9a5b6ff` (origin/main). Diff against it, never against a local `main`, which may be
  stale. Before stage 001, the branch's commits on top of it are plan documents only
  (`git diff --name-only 9a5b6ff HEAD` lists only `plans/`).
- **Design:** [proposal.md](proposal.md): the `statusNotices` row table, the allowed markup
  replacements, the `SuperdoughBackend` and `AppOptions` shapes, and the proofs.
- **Evidence:** [reverify.md](reverify.md), with file and line at `9a5b6ff`. Its **Decisions**
  section is binding.
- **Stages, in order, one commit each:**
  1. [001-status-notices.md](001-status-notices.md): (b) the pure `statusNotices`;
  2. [002-superdough-backend.md](002-superdough-backend.md): (c) the `SuperdoughBackend` type;
  3. [003-app-options.md](003-app-options.md): (a) `createApp(options)`.

  Sequential: finish each stage's gate before starting the next. 002 and 003 edit the same
  lines of `app.ts`.
- **Background reading:**
  - `AGENTS.md`: the Layout bullets for `src/lib/audio/` (`:84-90`), `play-arbiter.ts` and
    `shell.ts` (`:120-128`); the Rules on tests (`:363`), `runtime.ts` and superdough (`:374`),
    format (`:379`), styling and contextual help (`:445-453`), and the headless host
    (`:454-`);
  - `docs/writing-contextual-help.md`. No hint is written or changed here. The three action
    hints and the "audio suspended" hint stay in the component, byte for byte;
  - [plans/0133-timeline-model/proposal.md](../0133-timeline-model/proposal.md), the model for
    the whole-file markup proof and the literal counts.

## Rules

- **No visible change and no sound change.** All of these stay as they are:
  - the DOM, classes, attributes (`title`, `role`, `data-tone`) and the notice order;
  - every copy string and every help hint;
  - which backend plays, and what `graph()`, `masterLatencySec()` and `prepareAudition()` return
    and when;
  - `createApp()`'s defaults.
- **Frozen in every stage:** `src/lib/strudel/runtime.ts` (a host-bundle input, so an edit would
  need `host:build`), `src/lib/audio/engine-notices.ts`, `src/lib/recording/memo-ui.ts`,
  `src/components/ui/*`, `src/App.svelte`, `src/components/Settings.svelte`, every Rust file,
  every existing test file not named in a stage, and `tests/engine-notices.test.ts` and
  `tests/memo-ui.test.ts` in particular.
- **Existing tests are frozen** except where a stage names the exact edit: 002 deletes one
  self-test and its import from `tests/superdough.test.ts`; 003 rewrites the 12 calls in
  `tests/app.test.ts` and adds two type imports. No assertion or expected value changes
  anywhere.
- **New logic ships with tests** in the same commit.
- **Run `bun run format`** before each commit (frontend files). There are no Rust changes, so
  no `cargo fmt` and no cargo gate.
- **Never start the app** (`bun run tauri dev`, `app:signed`, `bun run dev`), and never start
  playback.
- **Test cadence.** While working, run only the stage's targeted test files. When the stage is
  done, run its full gate once, in order. Log each command and read the log; never re-run a
  command to read its output again.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan, which also wrote the proof scripts there.
A later session sets `S` to its own scratchpad and recreates the scripts from the stage
appendices. `L` logs a command with its start time and exit code, under the stage's number
(`N=001`, `002` or `003`):

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
N=001
L() { log="$S/0136-$N-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

Every stage runs these, in order, after its commit. Each stage file adds its own proof steps
and its exact touched-file list.

| Step | Command | Pass |
|---|---|---|
| 1 | `L format bun run format`, then `L format-clean git diff --exit-code` | exit 0 each: format rewrote nothing that was not committed |
| 2 | `L lint bun run lint` | exit 0 |
| 3 | `L check bun run check` | exit 0, and svelte-check's summary line reads `0 ERRORS 0 WARNINGS`, as in the stage's base log (at `9a5b6ff`: `COMPLETED 1260 FILES 0 ERRORS 0 WARNINGS 0 FILES_WITH_PROBLEMS`; the file count grows with 001's two new files) |
| 4 | `L bun-test bun test` | exit 0; the log shows the stage's new tests passing, and `tests/theme-tokens.test.ts`, `tests/contextual-help.test.ts` and the host-bundle staleness test passing |
| 5 | the stage's proofs (stage file, Validation) | as the stage file says |
| 6 | `L touched git diff --name-only 9a5b6ff HEAD -- . ":(exclude)plans"` | exactly the cumulative list the stage file gives |
| 7 | `L agents git diff 9a5b6ff HEAD -- AGENTS.md` | only the hunks the stages so far planned |

If a gate fails, fix it, amend the stage's commit, and re-run the gate once.

**Each stage report lists:** the exit line of every log, the proof outputs (the literal table,
the markup and JS script lines), the names of the new tests, and svelte-check's summary line.

## STOP, and report to the user, on any of these

Do not work around them:

- any change to a copy string, a help hint, a tone, a `title`, the notice order or either/or
  pairs, a class or attribute, or the "audio suspended" button;
- a markup difference beyond the allowed replacements (prettier's reflow of one included), or
  a script difference beyond the stage's planned edits;
- a literal count that differs from its row;
- any runtime or sound change: a transpiled-JS difference beyond the planned replacements, a
  different backend playing, a different `graph()` or latency;
- an existing test that needs a changed assertion or expected value to pass, or a new test
  whose expected value would have to differ from what today's code computes;
- a needed edit to a frozen file, `runtime.ts` above all;
- svelte-check errors or warnings;
- a needed `host:build` or `engine:refs` (no touched file should need them).

Never loosen a check, and never re-record an expectation, to absorb a difference.

## Commits

- One commit per stage, with the message its stage file gives: plain Conventional Commits, no
  task or plan identifiers. End each with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed.
