# 0127: one stage list, planned once and walked by the build

## Goal

A voice's stages are decided in one place. Today `crates/strudeland-engine/src/chain.rs` writes
the stage list twice:

- `NodeTable::plan` / `plan_entry` name the stages and register the `nodes` entries;
- `build_entry` (and the head and tail of `Voice::build`) build the same stages, restating every
  presence condition.

A debug assertion compares the two lists by `StageKind` only. After this task:

- `NodeTable::plan` is the only code that decides which stages a voice has and what it
  registers. It records each stage as a `StageSpec`, which says what to build.
- `Voice::build` plans first and builds by walking the specs. Each spec goes through one
  `build_stage`, which makes exactly today's fallible calls in today's order.
- The FM matrix walk is planned once (`FmStep`s), and `apply_fm` walks the steps.
- Node table entries are named by a `NodeName` enum instead of string literals and `format!`.
- The structural tests check the plan directly instead of inspecting built stages.

**Nothing the engine does changes.** These stay the same, byte for byte:

- every render (offline, live, stems);
- every error text and the order of the throws;
- the "Available targets" log and its order;
- the `controls --json` dump, every `signatures` output, and the modulator targets;
- live timing (no work moves onto the output callback, and the build does the same work).

## Background

[The re-verification](0127-fx-entry-walk/reverify.md) has the evidence at `416caa6`. In short:

- **Two lists, one assertion.** `plan_entry` (chain.rs:917-1000) and `build_entry`
  (chain.rs:1175-1325) list the same ten steps per entry in the same order. `NodeTable::plan`
  (chain.rs:805-869) and `Voice::build` (voice.rs:1450-1680) do the same for the voice level
  (`Gain(0.3)`, envGain, the entries, Release, post). Every condition is written on both sides.
  The debug assertion (voice.rs:1627-1635) and its test twin `the_plan_names_the_built_stages`
  (chain.rs:3057) compare only the `StageKind` sequence.
- **The plan already runs for every voice in release.** `Voice::build` calls
  `NodeTable::plan` after building (voice.rs:1627), because the modulator loop needs the table.
  So every voice lists its stages twice today. Only the comparison is debug-only.
- **FM is walked twice too.** `plan_fm` (chain.rs:873-909) and `apply_fm` (voice.rs:780-829)
  both walk the matrix, track first use and decide the noise skip.
- **Visible orders.** The throw order shows in the error text. The registration order shows in
  the "Available targets" skip log. For the worklet synths `vib` is registered after the FM but
  built before it. `param_value` (voice.rs:1737-1777) maps `StageId`s onto positions in
  `self.stages`, so a `StageId` must equal its stage's index.
- **No test proves bit identity** with the previous build. The #121 A/B guard does, and it
  still builds at `416caa6`.

A dry run while planning, on the release binary at `416caa6`, confirmed that the guard's new
jobs reach what they are for. Examples:

- `y02` (an oscillator with vib and FM) logs `Available targets: source, vib, vib_gain, fm_1,
  fm_1_gain, fm_2, fm_2_gain, gain, post`;
- `y03` (supersaw, the same controls) logs `source, fm_1, fm_1_gain, fm_2, fm_2_gain, vib,
  vib_gain, gain, post`;
- `y07` (a chain entry with every key) logs all 23 of its keys, ending with `delay, delay_mix,
  room, room_mix`;
- `y34` (an `lfo` on the noise modulator's `fmh3`) is refused as `lfo:fmh3`;
- the throw ladder `y24`-`y32` reports `gain`, `Q`, `unknown vowel zz`, `value is not a
  finite`, the AudioParam `gain` text, the AudioParam `value` text, `pan`, the AudioParam
  `value` text, and `gain`, one job after another.

## Scope

In:

1. **The A/B guard** (stage 001), built before any engine edit and checked after every stage.
2. **`StageSpec` and the stage walk** (stage 002): `NodeTable` stores specs, `Voice::build`
   walks them through `build_stage`. Deleted: `build_entry`, `push_stage`, `push_shaper`, the
   debug assertion and `the_plan_names_the_built_stages`. `StageKind::of` becomes
   `#[cfg(test)]`: the replacement plan test keeps comparing built and planned kinds.
3. **`FmStep`** (stage 003): the planned FM walk, and `apply_fm` built from it.
4. **`NodeName`** (stage 004): typed node table names, with a cross-check against
   `modulate::TARGETS`.
5. **The structural tests on the plan** (stage 005).
6. **Docs:** the engine README's NodeTable paragraph (`README.md:191-198`) and the AGENTS.md
   `chain` line.

Out:

- **`Sends::connect`** (voice.rs:116-136) and the orbit-held delay, room and djf. Their
  registrations stay in the plan. Their build stays where it is, after `post`, with its
  late-error rule.
- **Vibrato and pulse-width flags read from the plan** (decided against; see Decisions).
- **`modulate::TARGETS`** stays a `&str` table, and `resolve`'s lookup stays by string.
- **The `mods()` tests**, and the `stages()` tests that check built values.
- **Refs, vectors, goldens, the host bundle, TypeScript.** No `PLANNER_FILES` edit and no JS
  change. So `engine:refs`, `engine:vectors`, `render:vectors`, `render:check`, `host:build`,
  `host:parity` and `ipc:build` are not run.
- **GLOSSARY.md.** `StageSpec`, `FmStep` and `NodeName` are engine-internal names, not song
  language.

## Technical design

### `StageSpec`

In `chain.rs`, beside `StageKind`:

```rust
/// One planned stage of a voice: what `build_stage` builds at this position of the chain.
/// Owned (keys and kinds only); the values are read from the `VoiceParams` at build time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageSpec {
    /// `Gain(0.3)` after a synth source (not behind a worklet synth or the bus sound).
    SynthGain,
    /// `envGain` with the amp (or bus) envelope.
    EnvGain,
    Transient(FxKey),
    /// The entry's gain stage, `Gain(gain * velocity)`.
    EntryGain(FxKey),
    /// One filter copy: a biquad (two specs for a 24 dB filter) or a ladder.
    Filter { entry: FxKey, kind: FilterKind, ladder: bool },
    Vowel(FxKey),
    Coarse(FxKey),
    Crush(FxKey),
    Shape(FxKey),
    Distort(FxKey),
    Tremolo(FxKey),
    Compressor(FxKey),
    Pan(FxKey),
    Phaser(FxKey),
    LocalDelay(FxKey),
    LocalReverb(FxKey),
    /// The `FXrelease` gain.
    Release,
    /// `post`, `Gain(postgain)`.
    Post,
}

impl StageSpec {
    pub fn kind(self) -> StageKind { /* SynthGain|EnvGain|EntryGain|Post -> Gain,
        Filter { ladder: true } -> Ladder, Filter { ladder: false } -> Biquad,
        Pan -> Panner, the rest by name */ }
}
```

`FilterKind` is `biquad::FilterKind`. Only `Lowpass`, `Highpass` and `Bandpass` appear in a
spec. `Notch` belongs to the phaser.

### `NodeTable`

- `stages: Vec<StageKind>` becomes `specs: Vec<StageSpec>`.
- `stage(id)` keeps its signature and returns `self.specs.get(id.0).map(|s| s.kind())`. So
  `modulate`'s `node_param` (modulate.rs:261) is unchanged.
- `stages()` is replaced by `specs() -> &[StageSpec]`. It is used in production (the walk), so
  it is not `#[cfg(test)]`.
- `stage_id(kind)` becomes `stage_id(spec)`. It pushes the spec and returns its index.
- `NodeTable::plan` and `plan_entry` stay the only code that decides stages and registrations.
  Each `t.stage_id(StageKind::X)` becomes `t.stage_id(StageSpec::X(key))`. Its conditions and
  registration order are unchanged, `vib` after the FM for the worklet synths included.
- The `NodeTable` doc and `plan`'s doc say the table is the plan `Voice::build` walks, not a
  description checked against the build.

### `Voice::build`

The order of everything fallible stays as it is. Only who decides presence changes.

1. `sound_name`, the bus index, `send_bus` and `SourceKind::of` run as today, with the same
   errors.
2. **New position:** `let node_table = NodeTable::plan(params, kind);`. It is pure and
   infallible. It ran later for every successful build before, so it now also runs for builds
   that throw. The risk is a panic on input the build would have refused; see Risks.
3. The ADSR, the context and the source build are unchanged (stage 003 passes the FM steps in).
4. **The head walk.** The specs up to and including `EnvGain` are built inline:
   - `SynthGain` pushes `Gain(SYNTH_GAIN)`;
   - `EnvGain` builds `bus_envelope`/`amp_envelope` (fallible, as today) and pushes it.

   The plan puts `[SynthGain,] EnvGain` first for every source. The old condition
   `worklet_peak.is_none() && !is_bus` is gone, replaced by the plan's (`!matches!(source,
   Supersaw | Pulse | Bus)`), which is the same set.
5. `pw_lfo`, `env_end` and the source stop are unchanged.
6. `EntryTimes` is built as today.
7. **The tail walk.** For every remaining spec:

   ```rust
   let id = StageId(stages.len());
   let stage = build_stage(spec, id, params, &times, &mut param_lfos)?;
   stages.push(stage);
   ```

   One push per spec, so `StageId == index` holds by construction.
8. The sends, `djf` and `dry` (the late-error block), the channel count, the teardown and the
   voice construction are unchanged. `node_table` moves into the `Voice`. The debug assertion
   is deleted.

### `build_stage`

```rust
/// Builds the stage `spec` names (each throwing as superdough does), reading its values from
/// `params` (the entry's through `params.entry(key)`, planned present). A filter's or the
/// phaser's LFO goes to `lfos`, connected to stage `id`. Not for the head specs (`SynthGain`,
/// `EnvGain`), which `Voice::build` builds with the source's envelope.
pub(crate) fn build_stage(
    spec: StageSpec,
    id: StageId,
    params: &VoiceParams,
    times: &EntryTimes,
    lfos: &mut Vec<ParamLfo>,
) -> Result<Stage, VoiceError>
```

It lives in `chain.rs` in `build_entry`'s place. Each arm is that stage's block of today's
`build_entry` (or of `Voice::build`, for `Release` and `Post`), moved unchanged, with
`if let Some(x) = e.x` replaced by `e.x.expect("planned")`. The fallible calls per arm:

| Spec | Calls, in order |
|---|---|
| `Transient` | none (the constructor) |
| `EntryGain` | `idl_float(gain_stage(e), "gain")` |
| `Filter`, ladder | `build_ladder`, then `filter_lfo` |
| `Filter`, biquad | `create_filter`, then `filter_lfo` (per copy, so per spec) |
| `Vowel` | `Vowel::new` |
| `Coarse`, `Crush` | `set_worklet_value` |
| `Shape`, `Distort` | `set_worklet_value` (value), then `set_worklet_value` (postgain) |
| `Tremolo` | `set_initial_value`, then `get_lfo` |
| `Compressor` | `Compressor::new` |
| `Pan` | `idl_float(2 * pan - 1, "pan")`, then `set_value` |
| `Phaser` | `build_phaser` |
| `LocalDelay` | `LocalDelay::new` (with `TailCut::at(end_with_release)`) |
| `LocalReverb` | `LocalReverb::new` (the same) |
| `Release` | `set_value`, `set_value_at_time`, `linear_ramp_to` |
| `Post` | `idl_float(params.postgain, "gain")` |
| `SynthGain`, `EnvGain` | `unreachable!("Voice::build builds the head")` |

The walk keeps the order between stages: chain entries first, main last, then Release and Post,
as `params.entries()` and `plan` give it. So the first throw of a hap is today's first throw.

`VoiceParams` gains `pub fn entry(&self, key: FxKey) -> Option<&EntryParams>` (`Main` is
`&self.main`, `Index(n)` is `self.fx.get(n)`). A filter's params come from a private
`filter_of(e, kind) -> Option<&FilterParams>`.

### `FmStep` (stage 003)

```rust
/// One FM matrix side as planned: the modulator number (1..=8), its position in creation
/// order, and whether this side creates it (its first use in the walk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FmSide { pub idx: usize, pub pos: usize, pub first_use: bool }

/// One entry of `applyFM`'s matrix walk as planned: the source side, then the target side
/// (`None` for the carrier), and whether the entry connects (its target is the carrier or a
/// modulator that is not noise).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FmStep { pub sides: [Option<FmSide>; 2], pub connects: bool }
```

- `plan_fm` pushes one `FmStep` per `params.fm.entries` element, in order, into
  `NodeTable::fm_steps`. Its registrations and `fm_noise` are unchanged (`fm_is_noise` keeps
  serving `modulate`).
- `apply_fm(params, steps: &[FmStep], carrier_freq, begin, ctx)` zips `params.fm.entries` with
  `steps`. Per side, in side order:
  1. `build_modulator` when `first_use`;
  2. `idl_float(e.amount, "gain")`;
  3. `idl_float(modfreq, "gain")`;
  4. `modulators[pos].freq = freq`.

  Then, when `connects`, it pushes the `FmConnection`. When not, it goes on: the freq set on
  the skipped noise target stays, as today. The modfreq per position is kept in a `Vec<f64>`
  beside `modulators`.
- `build_oscillator` and `build_worklet_synth` take the steps (`node_table.fm_steps()`) and pass
  them on. Their order of calls is unchanged, so `vib` is still built before the FM.
- Why this is equal. `connects` uses `noise_wave(&mods[idx - 1])`, the predicate
  `build_modulator` builds noise by (voice.rs:718). Positions count first uses in walk order in
  both. `steps.is_empty()` equals `params.fm.entries.is_empty()`, because `plan_fm` runs for
  every synth source, which is exactly the set of callers of `apply_fm`.

### `NodeName` (stage 004)

```rust
/// A key of superdough's `nodes` tracker that the plan or the modulator loop writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeName {
    Source, PwLfo, Vib, VibGain,
    /// `fm_<n>` and `fm_<n>_gain`, n in 1..=8.
    Fm(u8), FmGain(u8),
    Transient, Gain, Lpf, LpfLfo, Hpf, HpfLfo, Bpf, BpfLfo, Vowel, Coarse, Crush, Shape,
    Distort, Tremolo, TremoloGain, Compressor, Pan, Phaser, PhaserLfo, Delay, DelayMix, Room,
    RoomMix, Post, Djf,
    /// `lfo_<id>` / `env_<id>` (the modulator loop's).
    Mod(String),
}
```

- `as_str(&self) -> &str`: a literal per static variant. `Fm(n)` and `FmGain(n)` index the
  static tables `"fm_1".."fm_8"` and `"fm_1_gain".."fm_8_gain"`. `Mod(s)` returns `s`.
  `NodeName::filter(kind) -> (NodeName, NodeName)` gives `(Lpf, LpfLfo)` and so on.
- `NodeTable::entries` holds `(FxKey, NodeName, NodeList)`. `set`, `one` and `push_element` take
  a `NodeName`. **They match an existing entry by `as_str()`**, exactly today's string
  equality, so "the last writer replaces in place" keeps today's meaning by construction.
- `get(key, name: &str)` stays string-keyed, because `resolve`'s `node_key` fallback is the
  control's free text (modulate.rs:395). `keys(key)` returns `as_str()` in insertion order.
- `modulate::register` and `probe_tables` write `NodeName::Mod(format!("lfo_{id}"))` (and
  `env_`). `TARGETS` stays `&[(&str, &str, &str)]`.
- **The cross-check test** (in `modulate`'s tests, where `TARGETS` is visible). Every name the
  probe tables register on `Main` and `Index(0)`, except `Mod` names and `room`, appears in
  `TARGETS`' node column. `room` is exempt: it is the orbit's convolver on `Main` and the local
  reverb on a chain entry, and no param a control reaches. `TARGETS` reaches it through
  `room_mix`. The test also asserts that the probe tables register every static `NodeName`
  variant and every `Fm(1..=8)`/`FmGain(1..=8)`, through an exhaustive `match` helper. So a new
  variant fails to compile until the test lists it.

### Tests: replace, don't layer

The full list is in [005-tests.md](0127-fx-entry-walk/005-tests.md). In short:

- **Stage 002** keeps the suite green with the least change. The tests that called
  `build_entry` go through a test helper, `walk_entry`, that walks
  `NodeTable::entry_plan(FxKey::Main, …)` through `build_stage`. Their assertions stay.
  `the_plan_names_the_built_stages` is replaced by `the_plan_lists_every_source_and_stage`: the
  same six voices plus a chain entry with a local delay and reverb, their spec lists pinned, and
  the built stages' `StageKind::of` equal to the specs' `kind()` on the built voices.
  `build_entry_errors_leave_the_earlier_stages` becomes `the_walk_throws_at_the_first_planned_site`
  on `walk_entry`: one site per row, its text and the stages built before it.
  `VoiceParams::entry` gets a unit test.
- **Stage 003** adds `fm_steps_follow_the_matrix_walk` (positions, first use, the noise skip and
  `connects`, on the matrices of `fm_gain_node_table_last_writer`). The FM value tests stay.
- **Stage 004** adds the `NodeName` text test and the `TARGETS` cross-check. The tests that call
  `t.set(…, "gain", …)` take `NodeName`s.
- **Stage 005** moves the structural tests onto the pure plan: spec lists and registrations
  instead of `kind()` over built stages. It deletes `kind()`, `kinds()` and `voice_kinds()`.
  It adds `voice_build_throws_in_plan_order` (a chain entry before the main entry, the entries
  before Release, Release before Post, `pw_lfo` before the entries). Built-voice assertions stay where they check values (`output_channels`,
  `send_levels`, param values, envelope events). `node_table()` stays: `modulate`'s nine tests
  read the table the voice's own modulator loop wrote, beside `mods()` from the same run.
  `mods()` and the value tests over `stages()` are untouched.

## What must not change

Each item names the guard that proves it.

- **Sound bits.** Offline: the A/B guard's WAV sha256 for 276 jobs (the 181 refs and 95 extra
  jobs). Live and stems: its digest, 10 variants per job (7 live, 3 stems).
- **Error texts and throw order.** The guard's stdout (`errors` JSON, `renderMs` removed), the
  stderr text and its sha256, and the exit code, for every job. The throw-site jobs `y08`-`y32`
  make each site the first thrower, and `y46`-`y50` pin the order between sites with different
  texts (Release before Post, a chain entry before Release, a noise FM target's `gain`,
  `pw_lfo` before the entries, the local delay before the local reverb). Where two sites share
  a text (`value is not a finite`, the AudioParam `value` text), stage 002's walk test tells
  them apart by the stages built before the throw.
- **The "Available targets" order.** The guard's `y01`-`y07` (oscillator, both worklet synths,
  a chain entry, the main sends, every key), plus the refs jobs with modulators. Live logs go
  into the digest.
- **The `controls --json` dump and the modulator targets.** The guard compares the dump byte
  for byte (`modulatable_targets` plans the probe tables).
- **`signatures`.** The guard runs it on every job (`unsupported_targets` plans too; `y34` is a
  refused FM target).
- **Live timing.** Not byte-compared. The build does the same work: one plan per voice, as
  today. The debug-only comparison is gone, and stage 004 removes one `format!` per FM and
  filter registration. Nothing moves onto the output callback, and `strudeland-live` is not
  edited. `rt_alloc.rs` and `player.rs` run in `cargo test --workspace`.

## The A/B guard

[001-baseline.md](0127-fx-entry-walk/001-baseline.md) builds it: `$S/0127-baseline/ab.sh`, a
merge of #121's `ab.sh` and #122's `eq.sh`, with `BASE_SRC=416caa6`. The draft is
`$S/0127-draft/ab.sh` in the planning session's scratchpad, quoted in full in 001.

- **`baseline`** refuses unless `git diff --quiet 416caa6 -- 'crates/*/src/*'
  crates/strudeland-engine/tests/refs` holds. It builds the release engine binary and the #121
  digest crate (with the workspace `Cargo.lock`), writes the 95 extra jobs and records into
  `base/`:
  - `controls --json` with its stderr and exit code;
  - per job (refs and extra): `signatures` output with its exit code (`sig/<job>.txt`), and the
    render's exit code, WAV sha256, stderr sha256 and stdout without `renderMs`
    (`render.tsv`), plus the stderr text (`stderr/<job>.txt`);
  - the digest over the refs and over the extra jobs (`digest-refs.txt`, `digest-extra.txt`).

  It then checks:
  - every extra job's `signatures` exits 0;
  - `x04`-`x21` render with exit 0;
  - the dump holds 20 sounds and 91 targets;
  - `y01`-`y07` log "Available targets", and `y02`, `y03`, `y04` and `y07` log their exact
    lists;
  - `y08`, `y13` and `y46`-`y50` log their throws, and `y34` is refused;
  - no digest variant panics.

  Finally it snapshots `ab.sh`, `digest/` (without `target/`), `base/` and the HEAD sha to
  `~/.cache/strudeland-0127/`.
- **`check`** refuses if `tests/refs` differs from `416caa6`, or if `digest/src/main.rs`
  differs from the snapshot's copy (or the snapshot has none). It rebuilds both from the working
  tree, records into `cur/` and runs `diff -r base cur`. It exits 0 only with no difference.
- **`restore`** copies `digest/` and `base/` back from the cache (`ab.sh` itself is copied by
  hand first). `check` needs no baseline binary, so a cleared scratchpad costs one rsync.
- **Recorded twice:** `baseline`, then `check` straight away, must report 0 differences.
  Otherwise the guard varies run to run and cannot be used.

## Rejected

- **Borrowed specs** (`StageSpec<'a>` holding `&EntryParams`). The table outlives the build: it
  moves into the `Voice`, `modulate` keeps probe tables, and `unsupported_targets` plans from a
  map. A lifetime would tie `NodeTable` to `VoiceParams` everywhere. Keys and kinds are `Copy`,
  and `params.entry(key)` reads the values.
- **A visitor** (one trait with plan and build implementations). The plan and the build need
  different inputs: the plan needs no times and cannot fail, while the build is fallible and
  time-dependent. A trait would hide one `match` behind indirection. The spec list is the shared
  vocabulary.
- **Moving `Sends::connect` into the walk.** The sends connect after `post`. A throw there is a
  late error that keeps the voice playing into its sends, and the delay, room and djf they reach
  are orbit-held, not stages. Making them specs would change neither condition nor order, and
  it would mix two error rules in one walk.
- **Making `TARGETS` a `NodeName` table.** Its node column names nodes the plan never registers
  (`stretch`, `dry`, `bus`, `bmod`, `warp_lfo`, `wt_lfo`, `lfo`, `env`), and `resolve` falls back
  to the control's free text. It stays `&str`, and the cross-check test ties the two together.
- **Changing the `mods()` tests.** They test the modulator loop's results on built voices,
  which this task does not change.

## Dependencies

- After #126 (`416caa6`, `origin/main`). The local `main` is stale: diff against `416caa6`.
- The #121 digest crate (`~/.cache/strudeland-0121/digest/`). It still builds at `416caa6`: the
  re-verification ran `cargo check --offline` on it, and the copy is in the planning scratchpad
  as `0127-digest/`.

## Risks

- **The plan now runs for builds that throw.** It used to run only after a successful build,
  and in `unsupported_targets`/`probe_tables`. It must be total on any parsed `VoiceParams`. It
  indexes `params.fm.mods[idx - 1]` with `idx` in 1..=8 from the parsed matrix, and calls
  `filter_lfo_props` with plan times. Both are total on parsed params. `never_panics` and the
  guard's throw jobs run every such path, and a panic there is a STOP.
- **A spec arm that reads the wrong entry.** `build_stage` reads `params.entry(key)`. A spec
  planned under one key and built from another would build other values. The tests cover it:
  stage 002's `walk_entry` uses `Main`, and the chain-entry tests (`a_chain_entry_with_every_key…`'s
  gains `[0.25, 0.4, 0.5]`, `a_chain_entry_gain_stage`, `the_24db_doubling…`'s entry drive) build from `Index(n)`. So does
  the guard's `y07`, `y13`, `y21`, `y22`, `y39`, `y40`, `y47` and `y50`.
- **`Filter` per copy.** The 24 dB copies were one loop iteration each. They are now two specs,
  built the same way (`create_filter`, then `filter_lfo`, per copy). `y45` and the refs'
  24 dB jobs check the bits.
- **Shared error texts.** The guard cannot tell `coarse` from `shapevol` by text (`value is not
  a finite …`). Stage 002's walk test pins the number of stages built before each throw.
- **The scratchpad is per session and can be cleared.** The guard snapshots to
  `~/.cache/strudeland-0127/` and has a `restore` command. If the cache is lost too, 001 gives
  the rebuild from `416caa6`.

## Complexity

Medium. One crate, about six files:

- `chain.rs`, `voice.rs`, `modulate.rs` and `params.rs`, with their tests;
- the engine README and AGENTS.md.

No new behaviour. There are five sequential stages: 001 sets up the guard and commits nothing,
and each of 002-005 is one commit, green on its own.

## Decisions

1. **The head is built in `Voice::build`, the tail by `build_stage`.** EnvGain needs the
   source's ADSR, the worklet peak and the bus flag, and `pw_lfo` and the stop must run between
   envGain and the entries. `build_stage`'s head arm is `unreachable!`, and the plan test pins
   that every plan starts `[SynthGain,] EnvGain`.
2. **`build_stage` returns the `Stage`; the walk pushes it.** One push per spec makes
   `StageId == index` structural, so no assertion is needed.
3. **Values come from `params`, not the spec.** Specs are keys and kinds. Presence was checked
   by the plan, so `expect("planned")` marks each read.
4. **Vibrato and the pulse-width LFO keep their own guards.** `build_vibrato` and
   `pulse_width_lfo` read `params.vib` and `params.pwsweep` to build the values. Passing the
   plan's flag in would add a parameter to three builders and an `expect` per value, and would
   remove one predicate each. That is not simpler. The plan tests (`worklet_synth_node_table_entries`,
   the plan half of `the_pulse_width_lfo`) and the guard (`y02`-`y04`, `y41`-`y44`) pin both
   sides.
5. **`fm_noise` stays** beside `fm_steps`. `modulate` asks `fm_is_noise(pos)`, and deriving it
   from the steps would be a search per call.
6. **`NodeName` matches by `as_str()`.** Typed construction, string semantics. A `Mod` name
   cannot collide with a static one: static names never start with `lfo_` or `env_`.
7. **`node_table()` stays** (see Tests).
8. **The README and AGENTS.md change in stage 002**, where `NodeTable` becomes the plan the
   build walks. Stage 004 adds `NodeName` to the README sentence, and stage 005 re-reads both.

## Open questions

None. A review that wants vibrato or the pulse width read from the plan is a follow-up, after
this task.
