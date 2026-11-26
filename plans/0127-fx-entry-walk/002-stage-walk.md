# Stage 002: `StageSpec`, and `Voice::build` walks the plan

## Goal

`NodeTable::plan` is the only code that decides a voice's stages. It records each as a
`StageSpec`. `Voice::build` plans first and builds by walking the specs. Every entry-level
stage, Release and Post are built through one `build_stage`, which makes today's fallible
calls in today's order.

Deleted: `build_entry`, `StageKind::of`, the debug assertion in `Voice::build` and
`the_plan_names_the_built_stages`.

The design is in the proposal: [StageSpec](../0127-fx-entry-walk.md#stagespec),
[NodeTable](../0127-fx-entry-walk.md#nodetable),
[Voice::build](../0127-fx-entry-walk.md#voicebuild) and
[build_stage](../0127-fx-entry-walk.md#build_stage), with the per-arm call table.

## Prerequisites

- Stage 001 done: `$S/0127-baseline/base/` exists, and its immediate `check` reported 0
  differences.
- `git diff --stat 416caa6 -- crates` is empty.

## Files

- `crates/strudeland-engine/src/chain.rs`: `StageSpec`, `NodeTable`, `plan`, `plan_entry`,
  `build_stage` (replacing `build_entry`), `filter_of`, and the tests.
- `crates/strudeland-engine/src/voice.rs`: `Voice::build`, the imports, the `stages` doc, the
  `apply_fm` doc comment (it names `NodeTable::plan`, unchanged), and the tests.
- `crates/strudeland-engine/src/params.rs`: `VoiceParams::entry` and its test.
- `crates/strudeland-engine/README.md:191-198` and `AGENTS.md` (the `chain` item in the engine
  layout line).

`modulate.rs` does not change: `table.stage(id)` keeps its signature, and the plan has no
`stages()` caller there.

## Steps

1. **`VoiceParams::entry`** (params.rs, beside `entries()`):

   ```rust
   /// The FX entry `key` (`Main` is the hap's own), `None` past the chain.
   pub fn entry(&self, key: FxKey) -> Option<&EntryParams>
   ```

2. **`StageSpec`** (chain.rs, after `StageKind`), with `kind()`, as the proposal gives it.
   Delete `StageKind::of` (its only callers are the debug assertion and
   `the_plan_names_the_built_stages`).
3. **`NodeTable`.**
   - `stages: Vec<StageKind>` becomes `specs: Vec<StageSpec>`. `empty()` changes with it.
   - `stage(id)` maps through `kind()`.
   - `stages()` becomes `specs()`.
   - `stage_id` takes a `StageSpec`.
   - Rewrite the struct doc: "the plan of a voice: its source kind, the stage specs
     `Voice::build` walks, which FM modulators are noise, and superdough's `nodes` tracker in
     insertion order". `NodeTable::plan` stays the only constructor besides `empty`.
4. **`plan` and `plan_entry`.** Change only the argument of each `stage_id` call. Keep every
   condition, every registration and their order. In order:
   - `SynthGain`;
   - `EnvGain`;
   - `Transient(key)`;
   - `EntryGain(key)`;
   - `Filter { entry: key, kind, ladder: params.ftype_ladder }`, once per copy. The `filters`
     array gains each slot's `FilterKind` (`Lowpass`/`lpf`, `Highpass`/`hpf`, `Bandpass`/`bpf`);
   - `Vowel(key)`;
   - the shaper array's kinds become `StageSpec::Coarse(key)` and so on;
   - `Tremolo(key)`, `Compressor(key)`, `Pan(key)`, `Phaser(key)`;
   - `LocalDelay(key)`, `LocalReverb(key)`;
   - `Release`, `Post`.

   Update `plan`'s doc: "the stage list `Voice::build` walks" replaces "the stage list it names
   is exactly the one `Voice::build` makes". Update `plan_entry`'s doc: drop "[`build_entry`]
   builds exactly these stages".
5. **`build_stage`** (chain.rs, in `build_entry`'s place), with the signature in the proposal.
   - Destructure `times` as `build_entry` did.
   - For an entry spec, `let e = params.entry(key).expect("planned");`.
   - Each arm is the matching block of `build_entry`, moved unchanged, with
     `if let Some(x) = e.x` replaced by `let x = e.x.expect("planned")` (`.as_ref()` for the
     `Option<…Params>` fields). Each arm returns its `Stage` and pushes no stage.
   - **`Filter`:**
     - `let f = filter_of(e, kind).expect("planned");`
     - a ladder: `build_ladder(f, e, begin, hold_end, now)?`, then `filter_lfo(f, times)?`,
       then `push_filter_lfo((id, ParamSlot::LADDER_FREQUENCY), lfo, lfos)`, and it returns
       `Stage::Ladder`;
     - a biquad: `create_filter(kind, f, e.fanchor, begin, hold_end, now)?`, then
       `filter_lfo(f, times)?`, then `push_filter_lfo((id, ParamSlot::BIQUAD_FREQUENCY), …)`,
       and it returns `Stage::Biquad`.

     Keep the ladder comment.
   - **`Phaser`:** `build_phaser(…)?`, then
     `lfos.push(ParamLfo { stage: id, slot: ParamSlot::BIQUAD_DETUNE, lfo })`, and it returns
     `Stage::Phaser(notch)`.
   - **`LocalDelay`/`LocalReverb`:** `TailCut::at(end_with_release)`, computed in the arm.
   - **`Release`:** the block from `Voice::build` (voice.rs:1588-1597), less its condition,
     returning `Stage::Release(g)`. It uses `times.hold_end`, `params.release_raw`,
     `times.end_with_release` and `times.now`.
   - **`Post`:** `Stage::Gain(GainNode::constant(idl_float(params.postgain, "gain")?))`.
   - **`SynthGain | EnvGain`:** `unreachable!("Voice::build builds the head")`.
   - `filter_of(e: &EntryParams, kind: FilterKind) -> Option<&FilterParams>`: private. It maps
     `Lowpass`, `Highpass` and `Bandpass` to the three fields, and `Notch` to `None`.
   - The doc comment says what `build_entry`'s did, per stage. Delete `build_entry`.
     `push_stage` and `push_shaper` stay only if something still calls them. Otherwise delete
     them; the walk pushes. `push_shaper` becomes `Stage::Shaper(Shaper::new(kind))` in the
     shaper arms.
6. **`Voice::build`** (voice.rs), as the proposal's numbered list says:
   - `let node_table = NodeTable::plan(params, kind);` right after `kind`.
   - **The head:** `let head = node_table.specs().iter().position(|s| *s == StageSpec::EnvGain).expect("planned") + 1;`.
     `for spec in &node_table.specs()[..head]` matches on the spec:
     - `SynthGain` pushes `Stage::Gain(GainNode::constant(SYNTH_GAIN))`;
     - `EnvGain` computes the envelope exactly as today and pushes it;
     - any other spec is `unreachable!("the plan starts with the head")`.

     Delete the `worklet_peak.is_none() && !is_bus` condition. `worklet_peak` is still read
     for the envelope peak.
   - The pulse `pw_lfo`, `env_end`, the stop and `stop_frame` are unchanged. So is
     `EntryTimes`.
   - **The tail:** `for &spec in &node_table.specs()[head..]`, push `build_stage(spec,
     StageId(stages.len()), params, &times, &mut param_lfos)?`. Delete the entry loop and the
     Release and post blocks they replace.
   - Delete the debug assertion and its comment. `node_table` moves into the `Voice` as before.
   - Update the `stages` field doc: "the built stages, one per planned `StageSpec`, in plan
     order". Drop `[build_entry]`.
   - Fix the imports: drop `StageKind` and `build_entry`; add `StageSpec` and `build_stage`.
7. **Docs.**
   - README, `README.md:196-197`: replace "`NodeTable` is superdough's `nodes` tracker, which the
     modulators resolve their targets on." with: "`NodeTable::plan` decides the stages once:
     the table holds the stage specs `Voice::build` walks (each built by `build_stage`, in
     superdough's throw order) and superdough's `nodes` tracker, which the modulators resolve
     their targets on." Keep the rest of the paragraph.
   - AGENTS.md, the engine layout line: "`chain` (the voice's stereo stage chain, FX entries,
     the node table)" becomes "`chain` (the voice's stereo stage chain, FX entries, the node
     table and the stage plan `Voice::build` walks)".

## Tests

The structural moves are stage 005's. This stage keeps every assertion and changes only how
the tests reach the code.

- **Added, `chain.rs`:** a test helper

  ```rust
  /// Walks the main entry of `params`, planned alone (`NodeTable::entry_plan(FxKey::Main, ..)`),
  /// through `build_stage`: the stages built before the first throw, the table, the LFOs, and
  /// the throw.
  fn walk_entry(params: &VoiceParams, times: EntryTimes)
      -> (Vec<Stage>, NodeTable, Vec<ParamLfo>, Result<(), VoiceError>)
  ```

- **Re-pointed, `chain.rs`:** each call of `build_entry` becomes a `walk_entry` call with the
  same params and times, and the same assertions:
  - `built()` (it returns `Err` on the throw, as before);
  - `build_entry_errors_leave_the_earlier_stages` (its first two cases). Its third case
    plans under `FxKey::Index(1)` and builds `Main`. It keeps the plan assertions
    (`NodeTable::entry_plan(FxKey::Index(1), …)`), and its build half becomes
    `walk_entry(…)` with `stages.len() == 2`, the same assertion;
  - `worklet_value_throw_sites`, `the_transient_stage_runs_from_its_begin_to_its_end`,
    `ladder_envelope_events_and_throw_order`, `tremolo_throws_stop_the_build`,
    `compressor_values_and_throw_sites` and `vowel_throw_site` (the direct callers at
    `416caa6`: chain.rs:2122, 2161, 2211, 2492, 2772, 3248, 3283). Where a test passes a
    `stages` vector and checks it after the throw, it reads `walk_entry`'s first element;
  - any other direct caller. `rg -n 'build_entry' crates/` must print nothing at the end.
- **Replaced:** `the_plan_names_the_built_stages` becomes `the_plan_lists_every_source_and_stage`,
  with the same six values and the same `Voice::build` call. For each one it asserts:
  - `voice.node_table().specs()`, the exact list below;
  - `voice.stages().len() == voice.node_table().specs().len()`;
  - `voice.node_table().source()`, as before.

  The lists follow from today's `plan` (`M` is `FxKey::Main`). Write them out; a test that
  disagrees with today's build is a STOP, not an edit:
  1. sawtooth with every effect: `SynthGain, EnvGain, Transient(M), EntryGain(M)`, then
     `Filter{M, Lowpass, false}` twice, `Filter{M, Highpass, false}` twice and
     `Filter{M, Bandpass, false}` twice (24 dB), then `Vowel(M), Coarse(M), Crush(M), Shape(M),
     Distort(M), Tremolo(M), Compressor(M), Pan(M), Phaser(M), Release, Post`;
  2. supersaw with a ladder: `EnvGain, EntryGain(M), Filter{M, Lowpass, true}, Post`;
  3. pulse: `EnvGain, EntryGain(M), Post`;
  4. `one`: `SynthGain, EnvGain, EntryGain(M), Post`;
  5. white noise with the sends and djf: `SynthGain, EnvGain, EntryGain(M), Post`;
  6. bus: `EnvGain, EntryGain(M), Post`.

  Also assert, for each, that the specs up to `EnvGain` are `[EnvGain]` or `[SynthGain, EnvGain]`
  and that no later spec is a head spec. That pins the head split `Voice::build` relies on.
- **Added, `params.rs`:** `entry_finds_each_planned_entry`. For a value with two chain entries:
  - every `(k, e)` of `entries()` has `std::ptr::eq(params.entry(k).unwrap(), e)`;
  - `params.entry(FxKey::Index(2))` is `None`.
- **Added, `chain.rs`:** `stage_spec_kinds`. Each spec's `kind()`, one assertion per variant
  (with `Filter` both ways). An exhaustive `match` in the test fails to compile on a new
  variant.
- **Unchanged:** every other test, `kind()` and `voice_kinds()` included (stage 005 deletes
  them).

## Gates

[000-prompt.md](000-prompt.md) "Test cadence", with `N=002`. Every log ends `exit 0`, and the A/B
log reports `0 differences`.

## STOP conditions

- Any A/B difference: a WAV, an error text or its order, a log order, the dump, `signatures`
  or a digest line.
- A re-pointed test needs a different expected value.
- `the_plan_lists_every_source_and_stage`'s lists disagree with the build.
- A panic anywhere: an `expect("planned")`, an `unreachable!`, `never_panics`.
- `git diff --stat 416caa6 -- crates/strudeland-live crates/strudeland-engine/tests/refs` is
  not empty.

## Commit message

```
refactor(engine): build a voice by walking its planned stages

NodeTable::plan listed a voice's stages and build_entry built them, each
restating every presence condition, with a debug assertion comparing the
two by kind. The plan now records a StageSpec per stage, Voice::build
plans first and walks the specs, and build_stage builds each one with the
same calls in the same order, so the throw order and the stage ids stay
as they were.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
