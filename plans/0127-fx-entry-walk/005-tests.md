# Stage 005: the structural tests on the plan

## Goal

The tests that check *which* stages a voice has, in *what* order, and what they register now
ask the pure plan (`NodeTable::plan` / `entry_plan`: its `specs()` and registrations). They no
longer build a voice and classify its stages with the `kind()` helper. Tests of *values* (param
values, envelope events, channel counts, sends, rendered output) keep building. The `kind()`,
`kinds()` and `voice_kinds()` helpers are deleted.

The walk's error-order test, `the_walk_throws_at_the_first_planned_site`, already landed in
stage 002. This stage adds its voice-level twin, `voice_build_throws_in_plan_order`.

This stage changes no production code. Replace each assertion; do not add a plan assertion
beside the built one it replaces.

## Prerequisites

Stage 004 committed, its A/B log at `0 differences`.

## Files

- `crates/strudeland-engine/src/chain.rs` (tests only).
- `crates/strudeland-engine/src/voice.rs` (tests only).
- `crates/strudeland-engine/README.md` and `AGENTS.md`: re-read only.

## Helpers

In `chain.rs`'s tests, beside `voice()`:

```rust
/// The plan of `v` (parsed with the `voice()` helper's duration and cps).
fn planned(v: serde_json::Value) -> NodeTable
```

It plans with `NodeTable::plan(&params, SourceKind::of(&sound_name(params.s.as_ref())).unwrap())`.
`voice.rs`'s tests get the same helper with its own parse arguments. `M` stands for
`FxKey::Main`, and `X(k)` for `StageSpec::X(k)`, below.

## Tests that move onto the plan

Per test: what becomes a plan assertion, and what stays built. "Registrations" means the
existing `entry(…)`, `get(…)`, `keys(…)` and `has_entry(…)` assertions, read from the planned
table instead of `voice.node_table()`. Their expected values are unchanged.

### `chain.rs`

- **`main_entry_registers_gain_filters_pan_post`.** On the plan:
  - the stage comment becomes an assertion: specs `[SynthGain, EnvGain, EntryGain(M),
    Filter{M, Lowpass, false} ×2, Filter{M, Highpass, false} ×2, Pan(M), Post]`, and for the
    second value `[SynthGain, EnvGain, EntryGain(M), Filter{M, Bandpass, false}, Post]`;
  - every registration.

  Stays built: the two `output_channels()` assertions (2 and 1).
- **`worklets_are_built_in_superdough_order_and_registered`.** On the plan:
  - the two `kinds` lists, as specs: `[SynthGain, EnvGain, Transient(M), EntryGain(M),
    Filter{M, Lowpass, false}, Coarse(M), Crush(M), Shape(M), Distort(M), Pan(M), Post]` and
    `[SynthGain, EnvGain, EntryGain(M), Crush(M), Post]`;
  - every registration, the bare `transient` node included.

  Stays built: `output_channels()` (2, then 1 with its message), and the crush
  `.value = null is 0` assertion on `stages()[3]` (index 3 is `Crush(M)` in the plan).
- **`ladder_slots_replace_the_biquads_and_register`.** On the plan: the specs `[SynthGain,
  EnvGain, EntryGain(M), Filter{M, Lowpass, true}, Filter{M, Highpass, true}, Filter{M,
  Bandpass, true}, Pan(M), Post]`, and the `lpf`/`hpf`/`bpf` registrations.

  Stays built: `output_channels()`, the ladder param values, their one event each, and the
  defaults.
- **`native_effects_build_in_superdough_order_and_register`.** Built through `built()` (the
  stage 002 walk). The `kinds` list becomes `table.specs()`: `[EntryGain(M), Filter{M,
  Lowpass, false}, Vowel(M), Crush(M), Tremolo(M), Compressor(M), Pan(M), Phaser(M)]`. Add
  `assert_eq!(stages.len(), table.specs().len())`, one built stage per spec.

  The registrations stay on `table`. The LFO slots and the channel-count scan stay on the built
  stages. `channels_out` is stage logic, not classification.
- **`a_chain_entry_with_every_key_builds_in_superdough_order`.** On the plan:
  - the 21-entry `voice_kinds` list as specs (`SynthGain, EnvGain, Transient(X0), EntryGain(X0),
    Filter{X0, Lowpass/Highpass/Bandpass, false}, Vowel(X0), …, Phaser(X0), LocalDelay(X0),
    LocalReverb(X0), EntryGain(M), Release, Post`, where `X0` is `FxKey::Index(0)`);
  - the `Index(0)` and `Main` key lists, the `delay`/`delay_mix`/`room`/`room_mix` entries and
    the `stage(StageId(16|17))` kinds.

  Stays built: the second value's gains `[0.25, 0.4, 0.5]` and its `has_entry` pair. Move the
  `has_entry` pair to the plan too: it is a registration.
- **`the_24db_doubling_is_the_haps_and_the_ladder_the_entrys`.** The counts become counts of
  `Filter { ladder: false }` and `Filter { ladder: true }` specs on the plan. They stay `2`,
  `1`, `(1, 1)` and `(0, 1)`. Stays built: the entry's drive (`2.0`).
- **`the_main_entry_never_builds_local_effects`.**
  - `local_delay(&v).is_none() && local_reverb(&v).is_none()` becomes "no `LocalDelay`/
    `LocalReverb` spec" on the plan of the first value, and "one each, under `Index(0)`" on the
    second;
  - the `delay` registration (`OrbitDelay`) moves to the plan.

  Stays built: both `send_levels()` assertions.
- **`release_stage_rule`.** The conditions move to the plan:
  - the `ks[len-3..] == ["phaser", "release", "gain"]` check becomes the last three specs
    `[Phaser(M), Release, Post]`;
  - every "no node" case: no `Release` spec;
  - the two "builds" cases (`FXrelease 0.02`, and a `null` release): a `Release` spec.

  Stays built: the release gain's events (`stages()[len - 2]`, where the plan says `Release`),
  the `null`-release event time, and the ramp-to-zero rendering loop.
- **`tremolo_condition_and_values`.** The conditions move to the plan (`entry_plan`):
  - `stages[1]` is a `Tremolo(M)` spec;
  - the `tremolo`/`tremolo_gain` registrations;
  - "no tremolo whatever the other keys": no `Tremolo` spec, no `tremolo` entry.

  Stays built (`built()`): every LFO and gain value, the sync cases and the depth table. The
  `kinds` list at the end (`["gain", "crush", "tremolo", "panner"]`) becomes the plan's specs.
- **`local_delay_conditions_and_values`.** The condition loop moves to the plan: for each
  `(k, bad)`, no `LocalDelay(Index(0))` spec and no `Index(0)` `delay` entry, and the base
  value has one. Stays built: the default delay time, the clamps, the wet and dry gains and
  their events, and the three throw cases.
- **The other `kind()` users** become plan reads at the same index:
  - `compressor_values_and_throw_sites` and `vowel_throw_site`: "built before the throw" is
    `&table.specs()[..stages.len()]` equal to the specs the old `kinds` list named
    (`[EntryGain(M), Crush(M)]` and `[EntryGain(M), Filter{M, Lowpass, false}]`);
  - `compressor_makes_the_voice_stereo_and_pan_takes_the_stereo_law`: the panner follows the
    compressor in the plan's specs. Its `output_channels()` and the panner law stay built;
  - `chain_entry_channel_rules`: `voice_kinds(&v)[3..6]` becomes `specs()[3..6] ==
    [Filter{X0, Lowpass, false}, LocalDelay(X0), EntryGain(X1)]`. The channel sequence stays
    built.
- **Delete** `kind()`, `kinds()` and `voice_kinds()`.
  `rg -n 'fn kind\(|fn kinds\(|fn voice_kinds\(' crates/strudeland-engine/src/chain.rs` prints
  nothing. The search is scoped to `chain.rs`: `voice.rs`'s test helper
  `fn kinds(v: &Voice) -> Vec<FilterKind>` (voice.rs:3042) reads built filters, and it stays for
  the built assertions below.

### `voice.rs`

- **`filters_follow_superdough_order_and_24db_doubles_them`.** The `ftype` table and the
  presence cases read the plan: the `FilterKind`s of its `Filter` specs, in order. Keep two
  built assertions, through `voice.rs`'s `kinds(&voice(…))`: the `all` value
  (`[Lowpass, Highpass, Bandpass]`), and the `all` value with `ftype: "24db"`
  (`[Lowpass, Lowpass, Highpass, Highpass, Bandpass, Bandpass]`), each equal to the plan's
  filter kinds. They pin `build_stage` turning the spec's kind into the biquad's, once per copy
  of a doubled filter too.
- **`worklet_synth_node_table_entries`.** Every assertion reads `planned(…)` instead of
  `voice(…).node_table()`. Same values.
- **`the_pulse_width_lfo`.** The three `pw_lfo` registration assertions (no LFO: `[None]`;
  with LFO: `[Some(PulseLfo)]`; other sounds: none) read the plan. Stays built: the LFO values,
  the end, the NaN throws (`TypeError`) and the rendered comparison.
- **`worklet_synths_connect_straight_to_their_envelope_in_stereo`.**
  `v.stages().len() == 3` becomes the plan's specs `[EnvGain, EntryGain(M), Post]` for supersaw
  and pulse. Stays built: the envelope peaks (`stages()[0]` is `EnvGain` in the plan) and
  `output_channels()`.

## The error-order tests

`the_walk_throws_at_the_first_planned_site` (chain.rs) and `entry_plan_registers_under_its_key`
landed in stage 002, in place of `build_entry_errors_leave_the_earlier_stages`. This stage does
not change them; it may only move them beside the other plan tests, under the same names, with
the same rows. Its rows sit on `walk_entry` and one entry, so they cannot see the order between
entries, Release and Post.

Add `voice_build_throws_in_plan_order` in `voice.rs`, through `try_voice`. Each case arms two
sites whose texts differ, so the text names the first thrower:

| Value | Error contains | Order it pins |
|---|---|---|
| `{ "s": "sine", "FX": [{ "pan": "x" }], "cutoff": 100, "resonance": "x" }` | `pan is not a finite` | a chain entry before the main entry |
| `{ "s": "sine", "FX": [{ "pan": "x" }], "FXrelease": Infinity }` | `pan is not a finite` | the entries before Release |
| `{ "s": "sine", "FXrelease": Infinity, "postgain": "x" }` | `linearRampToValueAtTime` | Release before Post |
| `{ "s": "pulse", "pwsweep": "x", "velocity": "x" }` | `'value' on 'AudioParam'` | `pw_lfo` before the entries |

`Infinity` is written `{ "$num": "Infinity" }` in the `json!` value, as voice.rs's tests
already do (`"release": { "$num": "Infinity" }`). These are the guard's `y13`, `y47`, `y46` and `y49`, which record the same texts at `416caa6`.

Two values are not used, on purpose:

- `{ "postgain": "x" }` alone throws `gain`, the same text as the main entry's gain stage, so
  it cannot show that Post comes after the entries;
- `{ "FXrelease": 0.5, "release": Infinity }` (`y23`) never builds a Release stage
  (`FXrelease` is not above the release), and its ramp text is the amp envelope's, in the
  head.

## Left as they are

- `node_table()` stays. `modulate`'s nine uses read the table the voice's own modulator loop
  wrote, next to `mods()` from the same run. A planned table plus a separate `run_loop` would
  split one observation into two runs.
- Every `mods()` test and every value test over `stages()`.

## Docs re-read

- README `:191-198`: the paragraph names the plan, `build_stage` and `NodeName` (stages 002 and
  004). It still says the chain is "built per FX entry in superdough's order", which stays true.
- AGENTS.md: the `chain` item names the stage plan.
- `rg -n 'build_entry|the plan names the built' crates/strudeland-engine AGENTS.md` prints
  nothing. `rg -n 'StageKind::of' crates/strudeland-engine/src` finds only the `#[cfg(test)]`
  impl and `the_plan_lists_every_source_and_stage`.

## Gates

[000-prompt.md](000-prompt.md) "Test cadence", with `N=005`. Also report the test counts, from
the engine log's `test result:` lines, before and after this stage. The count may drop where a
test merged, never because a case was dropped.

## STOP conditions

- Any A/B difference. This stage changes only tests, so a difference means production code was
  touched: every hunk of `git diff HEAD~1 -- crates/strudeland-engine/src` must sit inside a
  `mod tests`.
- A plan list or registration that disagrees with what the old built assertion said.
- A row of `voice_build_throws_in_plan_order` that disagrees, or a stage 002 error-order row
  that has to change.

## Commit message

```
test(engine): check the stage plan directly instead of built stages

The structural tests built a voice and classified its stages to check
which ones exist and in what order. They now read the planned specs and
registrations, which is what decides both, and keep building only where
they check values. A voice-level test checks that the chain entries,
Release and Post throw in plan order.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
