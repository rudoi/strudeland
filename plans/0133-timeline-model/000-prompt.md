# 0133 implementation entry point

- **Branch:** `llm/andrew/strudeland/0133-timeline-model`.
- **Base:** `b0229a8`. Diff against it, never against a local `main`, which may be stale. The
  branch commits on top of it before stage 001 are plan documents only
  (`git diff --name-only b0229a8 HEAD` lists only `plans/`).
- **Design:** [proposal.md](proposal.md). The input and output types, the moved help table and
  the allowed markup replacements are there.
- **Evidence:** [reverify.md](reverify.md), with file and line at `b0229a8`. Its **Decisions**
  section is binding. Read §1 (what the script derives) and §3 (what must stay identical)
  before you edit anything.
- **Background reading:**
  - `AGENTS.md`: the `src/lib/project/` bullet (`:129-138`), and the Rules on styling and
    contextual help (`:442-450`);
  - `docs/writing-contextual-help.md` (no hint is rewritten; it says why the texts are
    capitalised sentences);
  - `src/components/Timeline.svelte` (whole file, 516 lines);
  - `src/lib/project/timeline.ts` (whole file, 263 lines; `sectionItemHelp` at `:227-263` is
    the shape to follow);
  - `src/lib/project/song-playback.ts:29-60` (`PlaybackSong`, `PlaybackTransport`,
    `PlaybackState`), `src/lib/strudel/runtime.ts:119-125` (`RuntimeState.started`,
    `.structure`), `src/lib/project/session.ts:43-58` (`SessionState`),
    `src/lib/strudel/transport.ts:249-268` (`isLoopPass`, `canPlayEnding`);
  - `tests/timeline.test.ts` (fixtures `song` at `:43-50`, the `sectionItemHelp` table at
    `:264-319`).

## Rules

- **No visible change.** All of these stay as they are:
  - the DOM, classes, `data-*` attributes (`data-range-focus`, `data-cat`), `aria-*`;
  - every help hint (`help` props, `use:helpTarget` in `SectionBlock.svelte`) and its text;
  - every copy string, the two commit problem messages included;
  - focus (`showRangeOptions`), the pointer and keyboard handling, the menu state;
  - the rAF `position` effect and the stale position after a stop;
  - `<style>`.
- **Markup outside `<script>`** is byte-identical to `b0229a8` except the two replacements in
  the proposal ("Allowed markup replacements"). The gate's markup script proves it.
- **Only these files change:** `src/lib/project/timeline.ts`, `src/components/Timeline.svelte`,
  `tests/timeline.test.ts`, and `AGENTS.md` (the one `timeline.ts` entry at `:137-138`). Every
  other file is frozen, song-playback, transport, structure, session, `SectionBlock.svelte`
  and the `ui/` primitives included.
- **Existing tests are frozen.** No line of `tests/timeline.test.ts` before the new
  `describe('timelineView')` changes except the import list (add `timelineView` and its
  types). No other test file changes.
- **New logic ships with tests** in the same commit.
- **Run `bun run format`** before the commit (frontend files). No Rust changes, so no
  `cargo fmt`, and no cargo gate.
- **Never start the app** (`bun run tauri dev`, `app:signed`, `bun run dev`), and never start
  playback.
- **Test cadence.** While working, run only `bun test tests/timeline.test.ts`. When the stage
  is done, run the full gate once, in order. Log each command and read the log rather than
  re-running it.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan. A later session sets `S` to its own
scratchpad. `L` logs a command with its start time and exit code:

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
L() { log="$S/0133-001-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

| Step | Command | Pass |
|---|---|---|
| 1 | `L format bun run format`, then `L format-clean git diff --exit-code` (run after committing, or after `git add -A`, so it shows only what format rewrote) | exit 0 each: format rewrote nothing that was not committed |
| 2 | `L lint bun run lint` | exit 0 |
| 3 | `L check bun run check` | exit 0, and the same svelte-check error and warning counts as the base log `$S/0133-001-base-check.log` (001 step 1) |
| 4 | `L bun-test bun test` | exit 0. The log shows `tests/timeline.test.ts` (with the new `timelineView` tests), `tests/theme-tokens.test.ts` and `tests/contextual-help.test.ts` passing. `theme-tokens` scans `src/**/*.{svelte,css,ts}`, so `timeline.ts` is in its scope; the moved strings carry no colour, size, radius, z-index or duration literal. `contextual-help` covers `src/lib/help/targets.ts`, which is untouched |
| 5 | `L literals "$S/0133-literals.sh"` (written in 001 step 1) | exit 0 |
| 6 | `L markup bun "$S/0133-markup.ts"` (written in 001 step 1) | exit 0: the markup and the whole file each equal `b0229a8` with the planned edits |
| 7 | `L touched git diff --name-only b0229a8 HEAD -- . ":(exclude)plans"` (git sorts by path) | exactly `AGENTS.md`, `src/components/Timeline.svelte`, `src/lib/project/timeline.ts`, `tests/timeline.test.ts` |
| 8 | `L agents git diff b0229a8 HEAD -- AGENTS.md` | one hunk, inside the `src/lib/project/` bullet's `timeline.ts` entry |

Notes:

- `timeline.ts` is not in `crates/strudeland-host/js/host.inputs.json` and not in
  `PLANNER_FILES`, so no `host:build` or `engine:refs` is needed (`bun test`'s staleness tests
  would fail otherwise).
- **The stage report lists:** the exit line of every log; the literal-count table from step 5;
  the markup script's output; the names of the new tests; the svelte-check counts from base
  and step 3.

## STOP, and report to the user, on any of these

Do not work around them:

- any change to a help or copy string, a class, a `data-*` or `aria-*` attribute, the DOM, the
  focus, pointer or keyboard behaviour, or the rAF loop;
- a markup difference beyond the two allowed replacements, prettier's reflow of one included,
  or a script difference beyond 001 step 4's edits (the whole-file check);
- a literal count different from what the script expects;
- an existing test that needs a changed assertion or expected value to pass, or a new test
  whose expected value would have to differ from what today's component computes;
- a needed edit to a frozen file;
- svelte-check errors or warnings above the base counts;
- the destructured `$derived` being refused and the per-field fallback (001 step 4) refused too.

Never loosen a check, and never re-record an expectation, to absorb a difference.

## Commits

- One commit, with the message the stage file gives, plain Conventional Commits, no task or
  plan identifiers. End it with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed.
