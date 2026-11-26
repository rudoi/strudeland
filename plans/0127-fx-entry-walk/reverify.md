# 0127 step 0: re-verification on main 416caa6

The issue stands. The line numbers have moved, and one claim needed correcting.

## One stage list, written twice

| Site | Now |
|---|---|
| `plan_entry` | `crates/strudeland-engine/src/chain.rs:917-1000` (node names as string literals) |
| `NodeTable::plan` | chain.rs:805-869 (source, pulse `pw_lfo`, vib, FM, `Gain(0.3)`, envGain, entries, Release, post, orbit delay/room/djf) |
| `plan_fm` | chain.rs:873-909 |
| `build_entry` | chain.rs:1175-~1325 |
| `apply_fm` | voice.rs:780-829 (called from `build_oscillator` :1125 and `build_worklet_synth` :1346) |
| debug assert | voice.rs:1627-1635 (compares `StageKind`s only) |
| `unsupported_targets`'s plan | modulate.rs:635 |
| `kind()` test helper | chain.rs:1998 |
| `Stage` / `StageKind` / `StageKind::of` / `NodeTable` | chain.rs:239 / :632 / :653 / :682 |
| test twin of the assert | `the_plan_names_the_built_stages` chain.rs:3057 |

**Correction:** `NodeTable::plan` is not debug-only. `Voice::build` calls it for every voice in release, after building (voice.rs:1627), because the modulator loop needs the table (`connect_modulators` :1688). So every voice lists its stages twice. Only the comparison between the two lists is debug-only.

## Both sides use the same order today

Per entry, chain entries first and Main last:

1. transient
2. gain (always)
3. lowpass, highpass, bandpass. Each is a Ladder with the entry's `ftype_ladder`; otherwise a Biquad, doubled when the hap is 24 dB. Each also registers its `*_lfo`.
4. vowel (5 elements)
5. coarse, crush, shape, distort
6. tremolo (`tremolo` and `tremolo_gain`)
7. compressor
8. pan
9. phaser (`phaser` and `phaser_lfo`)
10. non-Main entries only: the local delay (when `delay_applies`) and the local reverb (when `room > 0`)

At voice level, `Gain(0.3)` is planned when the source is not Supersaw, Pulse or Bus, and built when `worklet_peak` is none and the voice is not a bus. Release is added when `fx_release > release_raw`, on both sides.

Each condition is restated on both sides:

- every stage's presence check
- the filter-LFO presence (`filter_lfo_props` with plan times, `filter_lfo` with real times)
- FM create order, the noise skip and the connection count
- vib
- pulse width
- the sends and djf (`Sends::connect`, voice.rs:116-136)

The assert covers only the `StageKind` sequence.

## Orders that are visible

- **Throw order.** `build_entry` makes its fallible calls in a fixed order, and that order shows in the error text: `idl_float` (gain), `build_ladder`/`create_filter` then `filter_lfo`, `Vowel::new`, `set_worklet_value`, the tremolo's `set_initial_value`/`get_lfo`, `Compressor::new`, the pan's `set_value`, `build_phaser`, `LocalDelay::new`/`LocalReverb::new`.
- **Registration order on the source side.** It is not always build order. For the worklet synths, `vib` is registered after the FM (chain.rs:832-841) but built before it (voice.rs:1343-1346). Registration order is the `Object.keys` order, which shows in the "Available targets" skip log.
- **StageIds.** `param_value` (voice.rs:1737-1777) maps `Stage`, `StageWet`, `StageLfo` and `VowelFilter` ids onto positions in `self.stages`, so a StageId must equal the stage's index.

## Consumers of the plan

- `Voice::build` (render_core.rs:428; offline and live share it since #121).
- `modulate::run_loop`, which registers `lfo_<id>`/`env_<id>`.
- `modulate::resolve`, `params_of` and `unsupported_targets`, which use the plan without building.
- `modulate::probe_tables` (:723), which feeds `modulatable_targets` and so the `controls --json` `modulatorTargets`.
- Tests through `NodeTable::entry_plan` (`#[cfg(test)]`).

Test-only accessor counts: `stages()` about 41, `node_table()` 27, `mods()` 38, `entry_plan(` 8.

## Sound

The repo's tests check fixed thresholds (`micro.rs`, `fixtures.rs`) or compare live against offline (`live_equals_offline`, stems). None proves bit identity with the previous build.

The #121 A/B guard proves it, and it still builds against main: `cargo check --offline` of the digest crate exits 0. This task reuses that guard (`~/.cache/strudeland-0121/`), merged with #122's `eq.sh` extras: the controls dump, per-job signatures, and the 45 sound jobs.

None of the 181 refs jobs exercises an "Available targets" skip log, so new extra jobs cover it:

- an lfo aimed at a missing node
- the throw order: `resonance:"x"` with a cutoff, `velocity:"x"`, a bad vowel, a NaN tremolodepth, `pan:"x"`
- an FM matrix with a noise target, plus `fmi`/`fmh` modulators
- a ladder with 24 dB
- a chain entry with every key, including the local delay and room
- a pulse with a NaN `pwsweep`
- worklet synths with vib and FM

No `PLANNER_FILES` edit and no JS change, so the engine refs, vectors, host bundle and goldens stay untouched.
