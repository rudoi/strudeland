# 001: one tempo read, `Runtime.tempoNow()`

## Goal

The toolbar and the `state` method show the same tempo they show today, from one pure formula
(`tempoAtBar`) instead of computed by hand in each: the Toolbar through `Runtime.tempoNow()`,
the `state` method on the `position()` it already reads. Nothing visible changes beyond the proposal's
accepted one-frame toolbar transients.

## Prerequisites

The [proposal](../0126-runtime-clock.md) ("Stage 001: `tempoNow`") and
[reverify.md](reverify.md) ("Tempo shown by hand"). Base `5605397`.

## Files

- `src/lib/strudel/structure.ts`: `Tempo`, `tempoAtBar`.
- `src/lib/strudel/runtime.ts`: `Runtime.tempoNow`, its implementation.
- `src/components/Toolbar.svelte`: reads `tempoNow`.
- `src/lib/control/handler.ts`: `state()` calls `tempoAtBar` on its `position()`.
- `tests/helpers/stub-runtime.ts`, `tests/editor-hooks.test.ts`, `tests/setup.test.ts`: the stubs
  gain `tempoNow` (for the `Runtime` type).
- `tests/runtime-backends.test.ts`: one new test.
- `GLOSSARY.md`: **Shown tempo**.
- `crates/strudeland-host/js/host.js`, `host.inputs.json`: rebuilt by `host:build` only.

## Steps

1. **`structure.ts`.** Next to `bpmFromCps`, add:

   ```ts
   /** cps (at the meter in force), quarter-note bpm and meter. */
   export interface Tempo { cps: number; bpm: number; meter: Meter }

   /**
    * The tempo an evaluate at `cps` and `meter` has at song bar `bar`: the bar's meter when the
    * structure changes meter, else `meter`. `bar` null: at `meter`. bpm is not rounded.
    */
   export function tempoAtBar(cps: number, meter: Meter, s: SongStructure | null, bar: number | null): Tempo {
     const at = bar !== null && s?.meterChanges ? meterAtBar(s, bar) : meter;
     return { cps: cpsAtMeter(cps, meter, at), bpm: bpmFromCps(cps, meter), meter: at };
   }
   ```

   `meterAtBar` is declared later in the file as a function declaration, so the order is fine.
2. **`runtime.ts`.** Add to the `Runtime` interface, after `position()`:

   ```ts
   /**
    * The tempo to show now: the last successful evaluate's cps and bpm, at the meter of the song
    * bar sounding while a song with meter changes plays, else at the evaluate's meter. It is the
    * scheduled tempo: after a live edit it changes at once, ahead of the audio. Changes nothing
    * (unlike `position`, it prunes no segments).
    */
   tempoNow(): Tempo;
   ```

   Implement it on the returned object next to `position`:

   ```ts
   tempoNow() {
     const s = state.structure;
     const bar = state.started && s?.meterChanges ? (locate(now(), s, segments)?.songBar ?? null) : null;
     return tempoAtBar(state.cps, state.meter, s, bar);
   },
   ```

   It must not call `position()` or `pruneAt`.
3. **`Toolbar.svelte`.** Remove `playingMeter`, `meter`, `bpm` and the effect at :24-37, and
   the `bpmFromCps`, `cpsAtMeter`, `meterAtBar` and `Meter` imports. Add:

   ```svelte
   // The tempo shown: sampled on every runtime change, and every scope frame while a song with
   // meter changes plays.
   let tempo = $state.raw(runtime.tempoNow());
   $effect(() => {
     const { started, structure } = $rt;
     tempo = runtime.tempoNow();
     if (!started || !structure?.meterChanges) return;
     return scopeClock.subscribe(() => (tempo = runtime.tempoNow()));
   });
   ```

   The readouts become `tempo.cps.toFixed(3)`, `String(Math.round(tempo.bpm))` and
   `meterString(tempo.meter)`; their labels and titles are unchanged.
4. **`handler.ts` `state()`.** Keep `const position = rs.started ? runtime.position() : null;`
   (the playhead uses it). Replace the `meter` computation with

   ```ts
   const tempo = tempoAtBar(rs.cps, rs.meter, rs.structure, position?.located?.songBar ?? null);
   ```

   so meter and playhead come from that one clock read, as today. Do not call
   `runtime.tempoNow()` here: the Toolbar is its only caller. Send `cps: tempo.cps`, `bpm:
   Math.round(tempo.bpm)`, `meter: meterString(tempo.meter)`. In the `structure.ts` import,
   replace `bpmFromCps`, `cpsAtMeter` and `meterAtBar` (no other use in the file) with
   `tempoAtBar`; `meterString` stays (:388).
5. **Stubs.** Each is needed only for the `Runtime` type: no handler path calls `tempoNow`.
   `stubRuntime`: import `tempoAtBar` from `structure.ts` and add

   ```ts
   tempoNow: () => {
     const s = get(store);
     const bar = s.started && s.structure?.meterChanges ? (pos.located?.songBar ?? null) : null;
     return tempoAtBar(s.cps, s.meter, s.structure, bar);
   },
   ```

   The `editor-hooks.test.ts` (:46) and `setup.test.ts` (:17-42) literals (`satisfies Runtime`):
   each gains `tempoNow: () => ({ cps: 0.5, bpm: 120, meter: { num: 4, den: 4 } })`.
6. **`GLOSSARY.md`**, under Song structure, after **Armed section**:

   ```
   **Shown tempo**

   The cps, bpm and meter the toolbar and `strudeland state` report: the last successful
   evaluate's tempo at the meter of the bar now sounding. A live edit shows its tempo at once,
   briefly ahead of the audio.
   _Avoid_: audible tempo (what is sounding), current tempo
   ```

## Tests

- **New**, after the `getCps follows the audible meter…` test (:590) in
  `tests/runtime-backends.test.ts`: `tempoNow reports the scheduled tempo at the audible bar`,
  with `fake()` and `MIXED_SONG`, the assertions listed in the proposal (Tests, Stage 001):
  4/4 values at once; after `tick(22)`, meter `{ num: 3, den: 8 }`, cps `toBeCloseTo(4 / 3, 10)`,
  bpm `120`; a live `evaluate(MIXED_SONG, { cps: 1.5 })` right there gives bpm `360` at once,
  meter `toEqual({ num: 3, den: 8 })` and cps `toBeCloseTo(4, 10)` (360 bpm over a 1.5-quarter
  bar; pinned as numbers, never as `cpsAtMeter` of the returned meter); after `rt.stop()`, `{
  cps: 1.5, bpm: 360, meter: { num: 4, den: 4 } }`; and `SONG` playing at `tick(25)` gives its
  own 4/4 tempo. If tick 22, or the evaluate right after it, is not in the 3/8 bar for
  `tempoNow` (its bar comes from `now()`, the same audible reference `getCps` uses), stop and
  report rather than moving the tick or the expectation.
- **Unchanged:** `tests/control-handler.test.ts` :170-205, :208-217 and :1189-1214 pass as they
  are: `state()` runs the real `tempoAtBar` on the stub's `position()`. No other test changes
  beyond the two `satisfies Runtime` literals gaining `tempoNow`.

## Gates

```sh
bun run format
bun run host:build > "$S/0126-001-host-build.log" 2>&1; echo "exit $?" >> "$S/0126-001-host-build.log"
bun test > "$S/0126-001-test.log" 2>&1; echo "exit $?" >> "$S/0126-001-test.log"
bun run check > "$S/0126-001-check.log" 2>&1; echo "exit $?" >> "$S/0126-001-check.log"
bun run lint > "$S/0126-001-lint.log" 2>&1; echo "exit $?" >> "$S/0126-001-lint.log"
bun run host:parity > "$S/0126-001-host-parity.log" 2>&1; echo "exit $?" >> "$S/0126-001-host-parity.log"
cargo test -p strudeland-host > "$S/0126-001-host-cargo.log" 2>&1; echo "exit $?" >> "$S/0126-001-host-cargo.log"
```

Read the logs instead of re-running. Also check by `rg`:

- `rg -n "cpsAtMeter|bpmFromCps|meterAtBar" src/components/Toolbar.svelte src/lib/control/handler.ts`
  finds nothing;
- `rg -n "tempoNow" src/` finds only `src/lib/strudel/runtime.ts` and
  `src/components/Toolbar.svelte` (the handler uses `tempoAtBar`);
- by reading the diff: `tempoNow`'s body calls neither `position()` nor `pruneAt`.

## STOP conditions

Stop and report, without committing, if:

- a control-handler pin (`cps`, `bpm`, `meter`, `playhead`) or a `state` wire example needs a
  new value;
- the new test needs a different value than the proposal states for today's behaviour;
- any existing timing, playhead, getCps or engine-switch test fails or needs a looser tolerance;
- `host:parity` reports a difference, or `host:build` changes anything beyond the runtime and
  structure modules' code;
- a file outside the list above needs an edit.

## Commit message

```
refactor(runtime): read the shown tempo from one runtime method

The toolbar and the control state method each computed the tempo at the
audible bar by hand. Both now use the pure tempoAtBar: the toolbar
through Runtime.tempoNow(), which prunes no segments, and the state
method on the position it already reads, so its meter and playhead
still come from one clock read. Both keep their own rounding.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
