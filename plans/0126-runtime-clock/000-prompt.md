# 0126 implementation entry point

- **Branch:** `llm/andrew/strudeland/0126-runtime-clock`.
- **Plan:** [the proposal](proposal.md). It is the contract: `tempoNow`, the
  `ClockPort`, the `LiveClock` interface, the call-site table, the tests, the rejected options
  and the decisions.
- **Base:** `5605397` (`origin/main`). Diff against it, never against a local `main` that may be
  stale.
- **Evidence:** [reverify.md](reverify.md), with file and line at `5605397`, and the user's
  decision to keep the scheduled tempo.
- **Style precedent:** `src/lib/strudel/timing.ts` and `transport.ts` (pure modules with a header
  comment saying what they decide, no Strudel import), and `plans/0123-app-shell/`.

## Stages

Sequential: stage 002 builds on stage 001 (`tempoNow`'s `now()` becomes the clock's through
`const now = liveClock.now`, with no edit to its body).
Run each stage's gate before starting the next.

1. [001-tempo-now.md](proposal.md): `tempoAtBar` and `Runtime.tempoNow()`; the Toolbar reads
   `tempoNow`, the `state` method applies `tempoAtBar` to its own `position()`; the stubs; one
   runtime test; a glossary term.
2. [002-live-clock.md](proposal.md): `live-clock.ts` with its port, snapshot and edit;
   `runtime.ts` moved onto it; `tests/live-clock.test.ts` replacing the clock-maths end-to-end
   tests; the `AGENTS.md` layout line (no glossary term: the live clock is internal).

## Rules

- **No visible change.** The toolbar readouts, the `state` values, the playhead, timing,
  drawing, sound and engine switching stay identical, apart from the one-frame toolbar
  transients the proposal accepts under Decisions. If anything else seems to need a change,
  stop and report it.
- **Boundaries.** No audible tempo (#156). No engine-router change (#157): `swapTo`, `stop`,
  `pendingSwap`, `desired`, the drain and the evaluate rollback keep their code; `performSwap`
  changes only the `from` it builds. No playhead change (#158). Not edited: `timing.ts`,
  `transport.ts`, `song.ts`, `metered-pattern.ts`, `event-edits.ts`, any recording, memo or
  playback module, any Rust, the wire examples, `protocol.ts`.
- **Replace tests, don't layer them.** Stage 002 deletes or shrinks the end-to-end clock tests
  it replaces (proposal, Tests); the integration tests stay.
- **Host bundle.** `src/lib/strudel/runtime.ts` and `structure.ts` are listed in
  `crates/strudeland-host/js/host.inputs.json`, and `live-clock.ts` will be (runtime imports it).
  Each stage ends with `bun run host:build`; then `bun run host:parity` (needs Chromium: `bunx
  playwright install chromium` once) and `cargo test -p strudeland-host`. `host.js` and
  `host.inputs.json` are written only by `host:build`.
- **No `PLANNER_FILES` change** (`scripts/engine/provenance.ts:30`): `src/lib/strudel/timing.ts`
  is the only planner file near this work and it is not edited, so `engine:refs` is not run. If
  a diff touches a planner file, stop.
- **Don't run** `ipc:build`, `render:check`, `engine:*` or `render:vectors`. Nothing they cover
  changes.
- **Test cadence.** While working, run only the file you are changing (`bun test
  tests/runtime-backends.test.ts`, `tests/control-handler.test.ts`, `tests/live-clock.test.ts`).
  When the stage is done, run each gate once, logging output and the exit code, and read the log
  rather than re-running. `S` is your session's scratchpad.
- **Never start playback for the user.** Manual checks are listed for the PR; the user runs them.
- **Commits are plain Conventional Commits** with the message the stage gives, no task or plan
  identifiers in the message or in code comments, ending with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`.
- **Follow-ups already filed:** #156, #157, #158. Cite them in the PR body; file anything new
  with `keitora task new`.

## Manual smoke (PR test plan; the user runs it on `bun run tauri dev`)

1. Open a mixed-meter song and play it. Watch the toolbar cps and meter switch at each meter
   change; bpm stays put.
2. While it plays, edit `meta.bpm` and re-evaluate: bpm and cps change at once (the scheduled
   tempo, as today), the meter keeps following the bars.
3. While it plays, edit a `meterChanges` entry and re-evaluate: the meter readout follows the
   new map as the playhead reaches it; no jump in the playhead or the sound.
4. Switch `meta.engine` between `'superdough'` and `'rust'` while playing: the switch lands at
   the next bar line with no doubled or missing note, and the readouts do not flicker.
5. Stop and start again, also from an armed section in another meter: the readouts show the
   reference tempo when stopped and the section's meter when playing.
6. `strudeland state` while stopped and while a mixed-meter song plays: `cps`, `bpm` and `meter`
   match the toolbar (cps unrounded, bpm rounded).
7. A constant-meter song with a tempo edit while playing: readouts as before (the playhead jump
   after the edit is #158's and stays).
