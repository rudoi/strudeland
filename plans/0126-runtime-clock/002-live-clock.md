# 002: the live clock, out of the runtime closure

## Goal

`src/lib/strudel/live-clock.ts` owns the live structure, the layout map, the tempo-edit history
and the phase offset, and the maths over them, reading the scheduler through a narrow
`ClockPort`. `runtime.ts` calls it instead of keeping loose variables, and the three places that
moved clock fields one by one (`editPattern`, `start`, `performSwap`) move one `ClockSnapshot`.
The clock maths gets unit tests over a fake port. Behaviour is byte-for-byte today's.

## Prerequisites

Stage 001 committed and green. The [proposal](../0126-runtime-clock.md) ("Stage 002: the live
clock", the call-site table, "Why `start` restores with `structure: liveClock.structure`") and
[reverify.md](reverify.md) ("The runtime is one closure"). Read `runtime.ts:332-404`,
`:441-541`, `:782-813`, `:869-1004` before editing.

## Files

- `src/lib/strudel/live-clock.ts` (new).
- `src/lib/strudel/runtime.ts`.
- `tests/live-clock.test.ts` (new).
- `tests/runtime-backends.test.ts`: :556 deleted, :590 shrunk (:540 stays whole).
- `AGENTS.md` (layout). No `GLOSSARY.md` term: the live clock is an internal module concept and
  the glossary is song language.
- `crates/strudeland-host/js/host.js`, `host.inputs.json`: rebuilt by `host:build` only (the new
  module joins the inputs).

## Steps

1. **Create `live-clock.ts`** with a header comment: what it decides (where the audible playhead
   is on the song's layout while tempo and meter edits are still in the scheduler's lookahead,
   and the cps Strudel's `getCps` reads), what it does not (segments belong to the transport,
   engines to the runtime), and that it imports no Strudel. Imports: `layoutClock` and
   `LayoutClock` from `./timing`; `buildStructure`, `cpsAtMeter`, `meterString`, `Meter`,
   `SongStructure` from `./structure`; the `Segment` type from `./transport`. Nothing else.
2. **Export** `ClockPort`, `ClockHistoryEntry`, `ClockSnapshot`, `LiveClock` and
   `createLiveClock(port: ClockPort): LiveClock` exactly as the proposal declares them, with a
   one-line doc comment per member saying which of today's names it is.
3. **Move the bodies verbatim** into `createLiveClock`, renaming only: `liveStructure` →
   `structure`, `clock` → `map`, `audibleHistory` → `history`, `r.scheduler` / `sch` → `port`
   (`sch.clock.duration` → `port.tickDuration`, `sch.getTime()` → `port.getTime()`, `sch.now()` →
   `port.now()`):
   - `toCycle`, `toClock` (:350-357);
   - `audibleReference` (:358-384), keeping `Number(port.lastEnd)` and
     `port.lastTick + 2 * port.tickDuration - port.getTime()` in that operand order;
   - `cpsNow` = `audibleCps` (:385-394);
   - `setLayout(next, anchor)` = `setLiveLayout` (:395-400) without `segments = next`;
   - `anchor()` = `scheduledAnchor` (:401-404);
   - `now()` = `Math.max(0, toCycle(audibleReference()))` (:782);
   - `edit(e)` = :494-515 in the same order (proposal), with `r.scheduler.started` →
     `port.started`, `r.scheduler.cps` → `port.cps`, `pending.cps` → `e.nextCps`,
     `pending.meter` → `e.meter`, `codeSetCps` → `e.codeSetCps`, `structure` → `e.structure`,
     and `setLiveLayout(segments, anchor)` → `setLayout(e.segments, anchor)`; it returns `anchor`.
     `history` is `readonly ClockHistoryEntry[]`, so the filter and push become `const next =
     history.filter((h) => h.end.clock > audible); if (next.at(-1)?.end.clock !== a.clock)
     next.push({ end: a, map, cps: port.cps, referenceMeter: structure?.meter }); history =
     next;` (behaviour-identical: the filter already made a new array);
   - `reset()`: `phaseOffsetSec = 0; history = [];` (from :805-808);
   - `snapshot()`: `{ structure, map, history, phaseOffsetSec }` (no copies);
   - `restore(s)`: assigns those four fields from `s` and nothing else (a `from` or `saved` with
     extra fields passes through without them being stored).

   Return arrow functions and getters (`get structure()`, `get map()`, `get phaseOffsetSec()`),
   never `this`, so methods can be passed unbound. Construction reads nothing from the port.
4. **`runtime.ts`.** In place of :334-404, keep `let segments` and `setSegmentSource(() =>
   segments)`, then build the port and the clock:

   ```ts
   // Getters: `r` is created below; nothing reads the port before it exists.
   const port: ClockPort = {
     get started() { return r.scheduler.started; },
     now: () => r.scheduler.now(),
     get lastBegin() { return r.scheduler.lastBegin; },
     get lastEnd() { return r.scheduler.lastEnd; },
     get lastTick() { return r.scheduler.lastTick; },
     get tickDuration() { return r.scheduler.clock.duration; },
     getTime: () => r.scheduler.getTime(),
     get cps() { return r.scheduler.cps; },
   };
   const liveClock = createLiveClock(port);
   function setLiveLayout(next: readonly Segment[], anchor?: { cycle: number; clock: number }) {
     segments = next;
     liveClock.setLayout(next, anchor);
   }
   ```

   Then apply the proposal's call-site table row by row: `editPattern` (the `previous` object
   holds `clock: liveClock.snapshot()` plus the five pattern fields; undo restores it; the
   :494-515 block becomes one `liveClock.edit(...)` call, passing the `structure` editPattern
   already read, whose result is the `anchor` the imminent check and nothing else uses; the
   imminent check reads `liveClock.map`; `playbackPattern` is built when `liveClock.map &&
   liveClock.structure`, as `meteredPlayback(seeded, liveClock.structure, () => liveClock.map,
   () => liveClock.phaseOffsetSec, …)`), `beforeEval`'s `setTime` and `setCpsFunc`, the `setCps`
   override, `now` (`const now = liveClock.now`), `pruneAt`, the transport boundary,
   `performSwap` (`cycleTime(liveClock.toCycle(handoff))` with `handoff` raw, and the `from`
   with `cycle: Math.max(0, handoff - early)`), `start` (its parameter type, save, `from`
   application and rollback), `cycleTime` and `drawingScheduler`. `tempoNow` needs no edit: its
   `now()` is now `liveClock.now`. Delete the `ClockHistory` type and every moved function and
   variable, and the imports only they used: `buildStructure`, `cpsAtMeter`, `meterString` from
   `./structure` and `type LayoutClock` from `./timing` (keep `layoutClock`: `query` uses it).
   Keep `start`'s comments, adjusting names.
5. **`AGENTS.md`**, in the `src/lib/strudel/` layout paragraph after `timing.ts`'s entry: ``
   `live-clock.ts` (the live clock: the audible playhead and cps across live tempo and meter
   edits, over a `ClockPort` the runtime builds on the scheduler; the edit history and phase
   offset, snapshotted as one value; no Strudel imports), ``.

## Tests

- **New `tests/live-clock.test.ts`**: the fake port and the eleven cases in the proposal
  (Tests, Stage 002). Structures come from `buildStructure` with the `MIXED_HEADER` meta
  (`meter: '4/4'`, one 3-bar section, `meterChanges` 3/8 at bar 2 and 4/4 at bar 3) and
  `INITIAL_SEGMENTS`. Expected numbers are derived by hand from seconds and cps, at
  `toBeCloseTo(…, 10)` wherever the replaced tests used it; never from the clock's own functions.
- **Replaced in `tests/runtime-backends.test.ts`:**
  - delete `repeated live tempo changes take effect…` (:556) and its `layoutClock` /
    `INITIAL_SEGMENTS` imports if now unused;
  - shrink `getCps follows the audible meter and tempo…` (:590) to the wiring, renamed `getCps
    reads the live clock`: `0.5` after the evaluate, `toBeCloseTo(4 / 3)` after `tick(22)`, then
    the same edit (`changed`, `{ cps: 1.5, meter: { num: 2, den: 4 } }`) and `tick(3)` →
    `toBeCloseTo(2)`, so the runtime's `edit()` wiring stays covered; `rt.scheduler.cps` after
    `rt.stop()`. Only the post-edit `4 / 3` check and the `tick(6)` → `0.75` check go (case 4
    covers them);
  - keep `editing a mixed meter preserves the play position…` (:540) whole, including
    `expect(rt.position().cycle).toBeCloseTo(anchor, 10)`: it is the runtime-level counterpart
    of case 5.
- **Fake port** (proposal, Tests): `tick()` does `lastBegin = lastEnd; lastEnd += tickDuration *
  cps; lastTick += tickDuration`, the fake time stepping with `lastTick` (zyklus's `phase`);
  after an `edit()` the test sets `port.cps = nextCps`, as `afterEval`'s `setCps` does. Case 4's
  values: `4 / 3` (cps 0.5 at 4/4, in the 3/8 bar), still `4 / 3` after the edit until its anchor
  sounds, then `cpsAtMeter(1.5, 2/4, 3/8)` = `2`, then `cpsAtMeter(1.5, 2/4, 4/4)` = `0.75`
  (write the numbers in the test, not the calls).
- **Unchanged:** every other test in the mixed-meter block (:452-712), the engine-switch and
  drain tests, stage 001's `tempoNow` test, `tests/runtime.test.ts`, and the control-handler
  tests.

## Gates

```sh
bun run format
bun run host:build > "$S/0126-002-host-build.log" 2>&1; echo "exit $?" >> "$S/0126-002-host-build.log"
bun test > "$S/0126-002-test.log" 2>&1; echo "exit $?" >> "$S/0126-002-test.log"
bun run check > "$S/0126-002-check.log" 2>&1; echo "exit $?" >> "$S/0126-002-check.log"
bun run lint > "$S/0126-002-lint.log" 2>&1; echo "exit $?" >> "$S/0126-002-lint.log"
bun run host:parity > "$S/0126-002-host-parity.log" 2>&1; echo "exit $?" >> "$S/0126-002-host-parity.log"
cargo test -p strudeland-host > "$S/0126-002-host-cargo.log" 2>&1; echo "exit $?" >> "$S/0126-002-host-cargo.log"
```

Read the logs; do not re-run a gate to read its result. Also check:

- `rg -n "@strudel|from '\.\./" src/lib/strudel/live-clock.ts` finds nothing (only `./timing`,
  `./structure`, `./transport`);
- `rg -n "audibleHistory|phaseOffsetSec =|liveStructure|scheduledAnchor|audibleCps" src/lib/strudel/runtime.ts`
  finds nothing;
- `git diff 5605397 -- src/lib/strudel/timing.ts src/lib/strudel/transport.ts scripts/engine/provenance.ts`
  is empty (no `PLANNER_FILES` change, so no `engine:refs`);
- `crates/strudeland-host/js/host.inputs.json` lists `src/lib/strudel/live-clock.ts`.

## STOP conditions

Stop and report, without committing, if:

- any kept test in `runtime-backends.test.ts` or `runtime.test.ts` fails, or passes only with a
  looser tolerance than `toBeCloseTo(…, 10)` / its current one;
- a `live-clock.test.ts` expectation derived by hand disagrees with the moved code (that is a
  difference between the old and new arithmetic, or a wrong derivation: find out which, do not
  adjust the expectation to the output);
- an engine-switch, drain or hand-off test changes (`performSwap`, `swapTo`, `stop`);
- `state` values, the playhead or the toolbar readouts would change;
- the `start` or `editPattern` restore needs a field the proposal does not list, or `segments`
  seems to belong in the snapshot;
- `host:parity` reports a difference, or `cargo test -p strudeland-host` fails;
- a file outside the list above, or a `PLANNER_FILES` file, needs an edit.

## Commit message

```
refactor(runtime): move the live clock out of the runtime closure

The live structure, layout map, tempo-edit history and phase offset, and
the audible-reference and getCps maths over them, move to live-clock.ts
behind a narrow port onto the scheduler. editPattern, start and the
engine switch save and restore one clock snapshot instead of loose
fields, and the clock maths is tested over a fake port.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
