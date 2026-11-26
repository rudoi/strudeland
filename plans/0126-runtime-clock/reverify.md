# 0126 step 0: re-verification on main 5605397

The issue stands. The line numbers below are corrected.

## The runtime is one closure

`src/lib/strudel/runtime.ts` is 1129 lines. `createRuntime` is one closure, from :238 to :1129.

**Live clock state:**

- `segments`, `liveStructure` and `clock` (:335-338).
- `ClockHistory` and `audibleHistory` (:341-347).
- `phaseOffsetSec` (:349).
- `toCycle` and `toClock` (:350-357).
- `audibleReference()` (:358-384). It reads Cyclist internals: `sch.now()`, `lastBegin`, `lastEnd`, `lastTick`, `clock.duration`, `getTime()` and `cps`.
- `audibleCps` (:385-394). Strudel's `getCps` points at it via `setCpsFunc`, at :546.
- `setLiveLayout` (:395-400) and `scheduledAnchor` (:401-404).
- `now` (:782), `pruneAt` and `position` (:785-795).
- The transport install resets all of this while stopped (:803-809).
- `cycleTime` (:971-981) and `drawingScheduler` (:994-1004).

**Snapshots that move clock fields as loose values:**

- `editPattern` saves 8 variables plus `schedulerPattern` (:469-479), undoes them (:480-493), then pushes history and rebases the phase (:494-515).
- `start` saves 5 values (:926-930), restores them (:958-966), and takes a 6-field `from` (:905-912).
- `performSwap` builds that 6-field `from` (:881-888).

**Engine routing** sits in the same closure: `activeId`, `desired`, `desiredBy`, `evaluates`, `pendingSwap`, `soundUntil`, `draining`, `swapTo`, `stop`, `performSwap`, and the evaluate request and rollback (:1026-1076). It is deferred to #157.

**Tests.** The clock is tested only end to end through `repl`, in `tests/runtime-backends.test.ts`:

- :556 (repeated live tempo changes against `layoutClock`);
- :590 ('getCps follows the audible meter and tempo across a change to both');
- the clock part of :540.

Everything else in the mixed-meter block (:452-712: `MIXED_HEADER` at :452, the first test at :459) is integration and stays.

## Tempo shown by hand

The Toolbar (`Toolbar.svelte:24-37, :93-99`) and the control handler (`handler.ts:297-322`, the `state` method) compute the same thing by hand:

- **meter:** `meterAtBar(structure, position().located.songBar)`;
- **cps:** `cpsAtMeter(rs.cps, rs.meter, meter)`;
- **bpm:** `bpmFromCps(rs.cps, rs.meter)`.

That is the scheduled tempo at the audible bar.

The audible tempo can differ:

- for about the scheduler lookahead after a mixed-meter edit;
- permanently when a constant-meter song uses `.cps()` on a hap.

**Decision (user, 2026-11-26):** keep showing the scheduled tempo. `tempoNow()` returns exactly today's values. The audible variant is #156.

**Other code that reads tempo uses the reference value on purpose**, and does not move:

- the recording controller and backing;
- `memo-player.ts`;
- the `song-playback.ts` start result;
- `event-edits.ts`;
- `memo-ui.ts`.

`StatusBar`, `Timeline`, `timeline.ts`, `piano-roll/move.ts` and `control/events.ts` read the static structure, not a live tempo.

**Tests that pin today's values**, all through `stubRuntime`:

- `tests/control-handler.test.ts:170-205`;
- `tests/control-handler.test.ts:208-217` (6/8 gives bpm 60);
- `tests/control-handler.test.ts:1189-1214` (3/8 gives cps 4/3, bpm 120).

The `state` wire example (`crates/strudeland-ctl/tests/wire/state.json`) and `methods.rs` do not change.

## Pre-existing, out of scope

After a constant-meter tempo edit, `position()` jumps back about 0.05 cycle. Fixing it is a visible change, filed as #158.
