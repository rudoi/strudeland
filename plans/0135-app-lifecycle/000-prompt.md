# 0135 implementation entry point

- **Branch:** `llm/andrew/strudeland/0135-app-lifecycle`.
- **Base:** `00feb37`. Diff against it, never against a local `main`, which may be stale. Before
  stage 001, the branch's commits touch plan documents only
  (`git diff --name-only 00feb37 HEAD` lists only `plans/`).
- **Design:** [proposal.md](proposal.md): the signatures, the lookup order, and three findings
  that correct reverify.
- **Evidence:** [reverify.md](reverify.md), with file:line at `00feb37`. Its **Decisions**
  section is binding. Read §1 (the two call tables) and §2 (the asymmetries) before any edit.
- **Background reading:**
  - `AGENTS.md`: the `src-tauri/` bullet (`:143-146`) and the Quit bullet (`:157-160`);
  - `src-tauri/src/lib.rs` (whole file, 126 lines);
  - `src-tauri/src/app_quit.rs:1-83` (`QuitState`) and its tests `:255-306`;
  - `src-tauri/src/control.rs:115-220` (`Bridge`), `:521-566` (`start`, `start_on`,
    `shutdown`), and the test `start_path_serves_and_closes_a_temp_endpoint` (`:1502-1556`);
  - `src-tauri/src/playback.rs:31-150` and `:186-190`, and the test helpers `:252-300`;
  - `crates/strudeland-record/src/takes/mod.rs:807-842` (`on_reload`, `shutdown`);
  - `crates/strudeland-record/src/takes/testing.rs:183-308` (`TakesRig`);
  - `crates/strudeland-record/src/takes/tests.rs:28-44` (how it builds a rig) and `:859-912`.

## Rules

- **No behaviour change.** These stay exactly as they are:
  - the reload calls, in the order `QuitState::detach`, `Bridge::detach`, `Takes::on_reload`,
    `playback::end_session`;
  - the exit calls, in the order `playback::end_session`, `Takes::shutdown`,
    `Bridge::close_server`;
  - the `main` label and `PageLoadEvent::Started` filter;
  - skipping a recorder or playback state that is not managed (today's `try_state`);
  - the panicking `state` lookups of `QuitState` and the bridge;
  - stderr (the callees' `eprintln!`s are not touched);
  - no events emitted;
  - `src/lib/ipc/bindings.ts`.

  Decisions 1 to 4 are recorded in docs only, never acted on.
- **Only these paths change:** `AGENTS.md`; `src-tauri/src/lib.rs`,
  `src-tauri/src/lifecycle.rs` (new), `src-tauri/src/control.rs`, `src-tauri/src/app_quit.rs`.
  Everything else is frozen, in particular:
  - `playback.rs`, `recording.rs`, `recording/rig.rs`, `export_audio.rs`;
  - `recording/transcript.rs` (its frozen regions must stay byte-identical, and the hook gate
    pins its hash);
  - the record crate;
  - every TS file and `SKILL.md`.
- **Edits outside 001's list are forbidden.** In `control.rs` the only edits allowed are these:
  - `fn close_server` becomes `pub fn close_server`, with its doc unchanged;
  - `shutdown` and its two doc lines are deleted;
  - `Manager` leaves the `tauri` import;
  - the `#[cfg(test)]` helper block is added.

  In `app_quit.rs` the only edit is the `#[cfg(test)]` helper block.
- **Existing tests do not change.** New logic ships with its tests in the same commit
  (001 step 5).
- **Run `cargo fmt --all`** before the commit. There are no frontend changes, so no
  `bun run format`.
- **Test cadence.** While working, run only `cargo test -p strudeland-app lifecycle`. When the
  stage is done, run the full gate once, in order. Log each command and read the log instead of
  re-running it.
- **Never start the app**, never start playback for the user, and never create a git worktree.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan. A later session sets `S` to its own
scratchpad and recreates the scripts from 001's appendix. `L` logs a command with its start time
and exit code. Run it from the repo root:

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
L() { log="$S/0135-001-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

| Step | Command | Pass |
|---|---|---|
| 1 | `L fmt cargo fmt --all --check` | exit 0 |
| 2 | `L clippy cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| 3 | `L test-app cargo test --locked -p strudeland-app` | exit 0. The log lists the five `lifecycle::tests::` names from 001 step 5 as `ok` |
| 4 | `L test cargo test --locked --workspace` | exit 0 |
| 5 | `L ipc bun run ipc:build`, then `L ipc-diff git diff --exit-code -- src/lib/ipc/bindings.ts` | both exit 0, and `ipc-diff` prints nothing |
| 6 | `L hooks "$S/0135-hooks.sh"` | exit 0: every line `PASS`, including the seven call counts and the two call-order lines for `lifecycle.rs` |
| 7 | `L literals "$S/0135-literals.sh"` | exit 0: no `FAIL` row |
| 8 | `L touched git diff --name-only 00feb37 HEAD -- . ":(exclude)plans"` | exactly `AGENTS.md`, `src-tauri/src/app_quit.rs`, `src-tauri/src/control.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/lifecycle.rs` |

Notes:

- No `PLANNER_FILES`, no file listed in `host.inputs.json`, and no change to a command, event or
  error kind. So `engine:refs`, `host:build` and `render:check` do not apply, and `bun test` has
  nothing new to check. Step 5 confirms that the bindings are unchanged.
- The `ipc:build` step rewrites `bindings.ts` in place. A non-empty `ipc-diff` is a STOP.
  Restore the file with `git checkout -- src/lib/ipc/bindings.ts` only after you have reported
  the diff.
- **The stage report lists:**
  - the exit line of every log;
  - the step 6 and 7 tables;
  - the five new test names;
  - the seam's line count (001 step 4: 18 as sketched, plus the 2-line `RefCell` import);
  - the call-order review from 001 Validation.

## STOP, and report to the user, on any of these

Do not work around them:

- any change to a call, its order, a filter or a skip, or any new call in either hook;
- a literal count different from its `after` column, or a hook-gate `FAIL`;
- a non-empty `bindings.ts` diff;
- an existing test that must change, or a new test that only passes once an assertion is
  loosened;
- an order seam of more than 20 lines (001 step 4). In that case drop the order test and report;
  do not grow the seam;
- an edit needed to a frozen file (the transcript, `playback.rs`, `recording/`, the record
  crate).

Never loosen a check, and never re-record an expectation, to absorb a difference.

## Commits

- One commit, with the message the stage file gives. Use plain Conventional Commits with no task
  or plan identifiers. End it with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`) and no bare `git stash`. To set work aside, make a
  WIP commit. Nothing under `$S` is committed.
