# Stage 003: the FM matrix walk, planned once

## Goal

`plan_fm` records the matrix walk it already does as `FmStep`s in the node table. `apply_fm`
builds from those steps instead of walking the matrix again. Today both sides track first use
and the noise skip.

The design, and why the result is equal, are in the proposal's
[FmStep](../0127-fx-entry-walk.md#fmstep-stage-003) section.

## Prerequisites

Stage 002 committed, its A/B log at `0 differences`.

## Files

- `crates/strudeland-engine/src/chain.rs`: `FmSide`, `FmStep`, `NodeTable::fm_steps` (the field
  and its accessor), `plan_fm`, and the tests.
- `crates/strudeland-engine/src/voice.rs`: `apply_fm`, `build_oscillator`,
  `build_worklet_synth`, the two call sites in `Voice::build`, and the doc comments that
  describe the walk.

## Steps

1. **Types** (chain.rs, beside `NodeTable`): `FmSide` and `FmStep`, as the proposal gives them.
2. **`NodeTable`.**
   - New field `fm_steps: Vec<FmStep>`, empty in `empty()`.
   - `pub fn fm_steps(&self) -> &[FmStep]`.
   - The struct doc gains "the FM matrix walk".
   - `fm_noise` and `fm_is_noise` stay unchanged.
3. **`plan_fm`.** Per matrix entry, in the same loop, collect `sides: [Option<FmSide>; 2]`:
   - a side with `idx == 0` is `None`;
   - otherwise `FmSide { idx, pos, first_use }`, where `first_use` is whether this side took the
     `None => …` arm that creates the modulator.

   After the side loop, push `FmStep { sides, connects }`. The registrations, `fm_noise`, the
   connection counter and their order are unchanged. Delete the `let _ = pos;` line: `pos` is
   now read.
4. **`apply_fm`:**

   ```rust
   fn apply_fm(
       params: &VoiceParams,
       steps: &[FmStep],
       carrier_freq: f32,
       begin: f64,
       ctx: &mut BuildContext,
   ) -> Result<Option<Fm>, VoiceError>
   ```

   - `if steps.is_empty() { return Ok(None); }`
   - `let mut modfreqs: Vec<f64> = Vec::new();`, indexed by position, beside `fm.modulators`.
   - `for (e, step) in params.fm.entries.iter().zip(steps)`. For each `Some(side)` of
     `step.sides`, in order:
     1. if `side.first_use`: `let (m, modfreq) = build_modulator(side.idx, &params.fm.mods, carrier_freq, begin, ctx)?;`,
        then push `m` and `modfreq`. Add `debug_assert_eq!(side.pos, fm.modulators.len() - 1)`,
        a cheap check of the planned position;
     2. `let amount = idl_float(e.amount, "gain")?;`
     3. `let freq = idl_float(modfreqs[side.pos], "gain")?;`
     4. `fm.modulators[side.pos].freq = freq;`

     These are today's calls in today's order, per side.
   - Then the connection:
     - the source is `step.sides[0].expect("the source is a modulator")`, with its `amount` and
       `freq` from step 1;
     - the target is `FmTarget::Carrier` when `sides[1]` is `None`, else
       `FmTarget::Modulator(pos)`;
     - push the `FmConnection` only `if step.connects`. Otherwise keep the comment (superdough
       logs "failed to connect FM ..." and goes on) and `continue`.

     The target side's freq set before the skip stays, as today.
   - Rewrite the doc comment: `apply_fm` walks the steps `NodeTable::plan` recorded
     (helpers.mjs:429-512). It builds each modulator at its planned first use and checks each
     side's `gainNode(amt)` and `gainNode(modfreq)` as superdough builds them. It skips a
     connection the plan marks as not connecting (a noise target).
   - Today's code never reads `amount`/`freq` back from a side other than the source, so keep
     only what the connection needs: the source side's values, and the target's position.
5. **The builders.** `build_oscillator` and `build_worklet_synth` take `fm: &[FmStep]` after
   `params` and pass it to `apply_fm`. Everything else in them is unchanged, the vibrato built
   before the FM included. In `Voice::build` both calls pass `node_table.fm_steps()`. The plan
   is already computed first (stage 002).
6. **Decided, not done:** vibrato and the pulse-width LFO keep their own guards (proposal,
   Decision 4). Do not route `params.vib` or `params.pwsweep` through the plan.

## Tests

- **Added, `chain.rs`:** `fm_steps_follow_the_matrix_walk`. It uses the matrices of
  `fm_gain_node_table_last_writer`, planned with `NodeTable::plan(&p, SourceKind::Osc)`:
  - `{ "fmi": 1, "fmi13": 3, "fmi2": 2 }`: three steps.
    1. `[Some({1, 0, true}), None]`, connects;
    2. `[Some({1, 0, false}), Some({3, 1, true})]`, connects;
    3. `[Some({2, 2, true}), Some({1, 0, false})]`, connects.
  - `{ "fmi": 1, "fmi13": 3, "fmwave3": "white" }`: the second step's target is `{3, 1, true}`,
    and it does not connect.
  - `{ "fmi13": 3, "fmwave3": "white", "fmi2": 2 }`: two steps, `[Some({1, 0, true}),
    Some({3, 1, true})]` (no connection) and `[Some({2, 2, true}), Some({1, 0, false})]`, which
    connects as connection 0 (`fm_2_gain` is `FmAmount(0)` in the existing test).
  - no FM: no step.
  - Supersaw and pulse plan the same steps as the oscillator for the same matrix. Bus, `one` and
    the noises plan none.

  The expected steps follow the matrix order in `fm_gain_node_table_last_writer`'s comment
  ("fmi (1 -> 0), fmi13 (1 -> 3), fmi2 (2 -> 1)") and agree with the registrations it asserts.
  **Add the record to `plan_fm` and this test before changing `apply_fm`**, and run it. If it
  fails then, the expectation is wrong (the walk has not changed yet): correct it from
  `plan_fm`'s output and say so in the stage report. If it fails after `apply_fm` changes, STOP.
  Also assert `steps.len() == params.fm.entries.len()` for every case.
- **Unchanged and green** (they check the built FM, which must not move): every FM value test
  in `voice.rs` (search `fn .*fm` in its tests), `fm_gain_node_table_last_writer`,
  `worklet_synth_node_table_entries` and `worklet_params_are_pulled_from_the_pause_frame`.
- **Nothing deleted.** No test reached `apply_fm`'s internals.

## Gates

[000-prompt.md](000-prompt.md) "Test cadence", with `N=003`. The guard jobs for this stage are
`y02`-`y04`, `y33`-`y37`, `y43` and `y44`, and the refs FM jobs. All of them are in the
0-difference check.

## STOP conditions

- Any A/B difference, an FM error text above all (`y35`-`y37`: `gain`, `frequency`, and `gain`
  after the vibrato).
- The `debug_assert_eq!` on a planned position fires.
- A built FM test needs a different expected value.
- `steps.len() != params.fm.entries.len()` for any voice. The zip would silently drop an entry;
  `fm_steps_follow_the_matrix_walk` asserts the lengths.

## Commit message

```
refactor(engine): plan the FM matrix walk once and build from it

plan_fm and apply_fm both walked the FM matrix, each tracking when a
modulator is first used and which connections a noise target skips. The
plan now records the walk as FmSteps and apply_fm builds from them, with
the same calls per side in the same order.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
