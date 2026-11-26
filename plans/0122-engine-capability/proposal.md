# 0122: what the engine plays, in its own module

## Goal

`crates/strudeland-engine/src/controls.rs` (3252 lines) splits into three modules, one per job:

- **`capability`** (public): what the engine accepts and refuses. It holds the allowlists, the
  unsupported detection behind the binary's `signatures` subcommand, and the `controls --json` dump.
- **`params`** (crate-only): the typed controls a voice reads, superdough's defaults and its
  mutation of a hap value.
- **`coerce`** (crate-only): the JS coercions (`Number(x)`, `String(x)`, truthiness, `Math.max`/`min`).

Two lists that today are hand-kept copies of the engine's own tables are then derived from them:

- the accepted sounds, from the table `SourceKind::of` reads;
- the modulator targets in the dump, from `modulate::TARGETS` through `resolve`.

A test pins every remaining hand list against the code that reads it.

**Nothing the engine does changes.** The following stay byte for byte the same:

- what it accepts and refuses;
- every `signatures` output;
- the `controls --json` dump;
- every render, refusal text and exit code;
- every live warning and the CLI's behaviour.

## Background

The [re-verification](reverify.md) has the evidence, with file and line at
`385724a`. In short:

- **One file, four jobs.** These live together:
  - the allowlists (`:14-656`);
  - the JS coercions (`:665-835`);
  - the typed params and superdough's mutation (`:837-1683`);
  - the unsupported detection and the dump (`:1685-1859`);
  - 1390 lines of tests.
- **The sound list is checked against nothing.** `ALLOWED_SOUNDS` (`:438-441`) is a hand copy of
  three things:
  - what `SourceKind::of` builds (`chain.rs:600-614`);
  - `OSC_SOUNDS` (`chain.rs:596-598`);
  - `NoiseKind::from_name` (`noise.rs:49`);
  - plus `SILENCE`.

  Nothing compares them.
- **The `'triangle'` default is written twice:** `sound_of` (`controls.rs:1656`) and
  `sound_name` (`voice.rs:2258`).
- **`MODULATOR_TARGETS` (91 names, `:564-656`) mirrors `modulate::TARGETS` (118 rows) by hand.**
  Only `controls_json` reads it.
- **`CHAIN_KEYS` is pinned only by its length** (`assert_eq!(CHAIN_KEYS.len(), 87)`, `:2713`).
- **AGENTS.md (`:212-213`) names a `signatures` module that does not exist.** The detection lives
  in `controls.rs`.

The research probes (logs `0122-research-probe.log`, `-chain.log` and `-sil.log` in the
planning session's scratchpad) showed that the derived sets equal today's lists:

- **Sounds.** The 17 names `SourceKind::of` accepts plus the 3 silence markers are exactly
  today's 20, in the same byte order.
- **Targets.** Resolving every `TARGETS` row against probe node tables gives exactly the 91 listed
  names. The 27 rows left out always skip or throw.
- **Chain keys.** Every `CHAIN_KEYS` entry changes `parse_entry_params`, with two exceptions:
  - `i`, which superdough reads and the engine ignores;
  - `delaysync`, which counts only when `delaytime` is absent.

  No allowed key outside `CHAIN_KEYS` changes it.

A further check, made while planning: a scratch copy of the crate with `controls` made
crate-private and only today's capability items re-exported passes
`cargo clippy --lib -- -D warnings`. The copy is
`0122-deadcode/` in the planning scratchpad. So narrowing `params` and `coerce` to the crate
makes nothing dead and trips no `private_interfaces` lint.

## Scope

In:

1. **The split** (stage 001): `capability.rs`, `params.rs` and `coerce.rs`, with their tests.
   `controls.rs` is deleted. Every import moves to the real module path, and there is no facade.
   This stage also fixes the stale docs: AGENTS.md's module list, the README and `controls.md`
   paths, and the code comments that name `controls`.
2. **The derivations** (stage 002):
   - `SourceKind::NAMES` and `DEFAULT_SOUND`;
   - `capability::allowed_sounds()`;
   - `modulate::modulatable_targets()`;
   - the tests that pin the remaining hand lists;
   - the README and `controls.md` text about the derived lists.
3. **The equality guard** (set up in stage 001 before any edit, checked after each stage). It
   compares outputs before and after: the `controls --json` dump, plus the `signatures` and
   `render` results of every `tests/refs` job and a set of extra sound jobs.

Out:

- **Any change to what is accepted, refused or rendered.** That includes the order of the
  `sounds`, `keys`, `chainKeys` and `modulatorTargets` lists in the dump. `{"s":["-"]}` (a
  non-string `s` whose JS string is a silence marker) stays accepted, and its hap still fails
  with `sound - not found`.
- **`js.rs`.** It holds the bit-exact ports checked against the JS vectors (`engine:vectors`).
  The coercions are checked by unit tests, so they go to their own module.
- **Deriving `ALLOWED_KEYS`.** It is the audit record, and `controls.md`'s two-way test keeps it
  honest.
- **`CHAIN_UNSUPPORTED_KEYS` and `MODULATOR_KEYS`.** The first is a short refusal list, and the
  second is already derived from `modulate::{BMOD,ENV,LFO}_KEYS`. Both move to `capability` unchanged.
- **The `user` fallback in `build_oscillator`** (`voice.rs:1103-1107`, `"triangle"` when `user`
  has no truthy `partials`). That is synth.mjs's own rule, not the default sound, and its
  literal stays.
- **TypeScript.** `src/lib/render/schedule.ts:166` says "controls.rs js_to_number" in a comment.
  `schedule.ts` is a planner file: it is in `PLANNER_FILES` (`scripts/engine/provenance.ts`) and
  in `crates/strudeland-host/js/host.inputs.json`. Editing even a comment there forces
  `engine:refs --write` and `host:build`. The comment is left as it is, to be fixed with the next
  planner edit (a follow-up task).
- **Refs, vectors, goldens and the host.** No engine output changes, and no TS or planner file
  is touched. So `engine:refs`, `engine:vectors`, `render:vectors`, `render:check`, `host:build`,
  `host:parity` and `ipc:build` are not run. `scripts/engine/provenance.ts` hashes no Rust file.

## Technical design

### The three modules

`capability` is the engine's answer to "can you play this?". It is a **deep module**. Callers
outside the crate use six names:

- `unsupported_signatures` and `UnsupportedSignature`: `strudeland-render`'s refusal and the
  binary;
- `controls_json`: the binary;
- `ALLOWED_KEYS`, `CHAIN_KEYS` and `ALLOWED_SOUNDS` (then `allowed_sounds()`): the integration
  tests.

Inside the crate, `live.rs` and `render.rs` use two more: `signature_of_with_bus` and
`unsupported_signatures_in`. Behind those sit the allowlists, rules 2a and 4, the FX-entry rules
and the modulator target check.

The other items that are `pub` today stay `pub`: `signature_of`, `CHAIN_UNSUPPORTED_KEYS`,
`MODULATOR_KEYS` and, until 002, `MODULATOR_TARGETS`. The split changes no visibility beyond
the ones it lists. `signature_of` has no non-test caller anywhere in the workspace; only
`capability`'s unit tests call it. Narrowed to `pub(crate)`, it would be dead code in non-test
builds and fail clippy. As a `pub` item of a public module it is not dead.

`capability.rs` (`pub mod`) gets, in this order:

| Item | From `controls.rs` | Visibility |
|---|---|---|
| module doc: what the engine plays, the audit in `controls.md`, the only unsupported detection | `:1-3`, rewritten | |
| `ALLOWED_KEYS` | `:12-435` | `pub` |
| `ALLOWED_SOUNDS` (stage 001), then `allowed_sounds()` (stage 002) | `:437-441` | `pub` |
| `CHAIN_KEYS`, `CHAIN_UNSUPPORTED_KEYS`, `MODULATOR_KEYS` | `:446-558` | `pub` |
| `MODULATOR_TARGETS` (stage 001 only) | `:560-656` | `pub` |
| `UnsupportedSignature`, `signature_of`, `signature_of_with_bus`, `signature_with`, `value_rules`, `fx_signature`, `unsupported_signatures`, `unsupported_signatures_in`, `controls_json` | `:1685-1859` | unchanged |

`params.rs` (`pub(crate) mod`) gets:

- `ControlError` (`:658-663`);
- everything from `// ---- notes and frequency` (`:837`) through `prepare_value` (`:1683`).
  That covers `note_to_midi`, `frequency_from_value`, `VoiceParams`, `EntryParams`, the
  tremolo, phaser, compressor, FM and filter params, `parse_voice_params`, `parse_entry_params`,
  `pitch_curve`, `gain_stage`, `sound_of`, `is_silence` and `prepare_value`;
- `SILENCE` (`:443-444`), moved beside `is_silence`, the rule that reads it.

Items keep their `pub` keyword; inside a crate-private module it means crate-visible. So
`SILENCE` becomes crate-only. Nothing outside the crate reads it: `git grep -nw SILENCE --
crates` at `385724a` finds only `controls.rs`. `lib.rs` does not re-export it, so no re-export
is needed. The silence markers stay public through `allowed_sounds()` and the dump.
`is_silence` (private today) becomes `pub(crate)`, because `capability`'s `signature_with`
calls it (`:1720`). The six filter key consts (`LOWPASS_KEYS`, `HIGHPASS_KEYS`,
`BANDPASS_KEYS` and the three `*_LFO_KEYS`) become `pub(crate)`, because `capability`'s
`chain_and_modulator_tables` test reads them. Their `FilterKeys`/`FilterLfoKeys` aliases change too if the compiler asks.

`coerce.rs` (`pub(crate) mod`) gets `// ---- JS coercions` (`:665-835`): `js_to_number`,
`js_to_number_full`, `js_truthy`, `js_to_string`, `js_max`, `js_min`, `parse_js_number` and their
private helpers.

The dependency runs one way, with no cycle:

- `capability` uses `params::{SILENCE, is_silence, sound_of}` and
  `coerce::{js_truthy, js_to_string}`;
- `params` uses `coerce::*` and nothing from `capability`.

Only test code reads back up: `params`'s `fm_control_names_follow_the_matrix_rule` imports
`capability::ALLOWED_KEYS`.

### The public API

Nothing outside the workspace uses the engine (it is not published). Inside the workspace, the
only users of `controls` are `strudeland-render/src/plan.rs`, the engine's binary and its
integration tests (`bin.rs`, `fixtures.rs`, `micro.rs`, `never_panics.rs`).

| Today | After 001 | After 002 |
|---|---|---|
| `controls::{UnsupportedSignature, unsupported_signatures, signature_of, signature_of_with_bus, CHAIN_KEYS, CHAIN_UNSUPPORTED_KEYS, MODULATOR_KEYS}` | `capability::…` | same |
| `controls::SILENCE` (`pub`, unused outside the crate) | crate-only `params::SILENCE` | same |
| root `ALLOWED_KEYS`, `controls_json` | root, from `capability` | same |
| root and `controls::ALLOWED_SOUNDS` | root and `capability::ALLOWED_SOUNDS` | root and `capability::allowed_sounds()` |
| `controls::MODULATOR_TARGETS` | `capability::MODULATOR_TARGETS` | deleted (`modulate::modulatable_targets()`, crate-only) |
| `controls::{js_*, VoiceParams, …, prepare_value}` (`pub`, unused outside the crate) | crate-only `coerce`/`params` | same |

`allowed_sounds()` is a function, not a static. `never_panics.rs` keeps its order: it reads
`let sounds = allowed_sounds();` once and indexes that.

### Derivations (stage 002)

**Sounds.** In `chain.rs`:

```rust
/// The sound a value without `s` plays (superdough's `s = 'triangle'` default).
pub const DEFAULT_SOUND: &str = "triangle";

impl SourceKind {
    /// Every `s` the engine builds and its kind (`getOscillator`'s periodic-wave types and their
    /// aliases, `user`, the noises, `one`, `supersaw`, `pulse`, `bus`).
    pub const NAMES: &'static [(&'static str, SourceKind)] = &[ /* 17 rows */ ];
    /// The kind of sound `s` (already defaulted), `None` for a sound the engine does not build.
    pub fn of(s: &str) -> Option<SourceKind> { /* lookup in NAMES */ }
}
```

- **`NAMES` lists today's match arms in order.** These are `one`, `supersaw`, `pulse`, `bus`,
  then the nine `OSC_SOUNDS` names in their order, then `white`, `pink`, `brown` and `crackle`.
  `OSC_SOUNDS` is deleted: its only reader was `of`. `NoiseKind::from_name` stays, because
  `fmwave` reads it.
- **`sound_of` and `sound_name` both use `DEFAULT_SOUND`.**
  - `params::sound_of` keeps returning a `Value::Str`.
  - `voice::sound_name` keeps returning a `String`.
- **`capability::allowed_sounds() -> Vec<&'static str>`** takes the `NAMES` names and
  `params::SILENCE` and sorts them in byte order. That gives today's 20, in today's order.
- **The sound check in `signature_with`** becomes
  `SourceKind::of(&s).is_some() || SILENCE.contains(&s.as_str())`, on the same bank-prefixed
  JS string as today. The `SILENCE` half is load-bearing. `is_silence` matches only a
  `Value::Str`, so `{"s":["-"]}` reaches the sound check as the string `-`. Today it passes there
  because `-` is in `ALLOWED_SOUNDS`.

**Modulator targets.** `modulate.rs` gains:

```rust
/// The `CONTROL_TARGETS` keys a modulator can resolve to an implemented param on some voice,
/// sorted: every `TARGETS` row resolved against the probe voices' node tables. The others skip
/// or throw wherever they are used. What `controls --json` prints as `modulatorTargets`.
/// The probe voices can miss a node some real voice has; `modulatable_targets_are_todays` is
/// the safety net for a row the probe skips.
pub(crate) fn modulatable_targets() -> Vec<&'static str>;
```

It is the research probe, kept as production code (`fn probe_tables() -> Vec<NodeTable>`,
private).

- **The FX map.** One map holds:

  ```json
  { "cutoff": 800, "lprate": 2, "hcutoff": 300, "hpdepth": 0.5, "bandf": 900, "bpdepth": 0.5,
    "vowel": "a", "coarse": 2, "crush": 4, "shape": 0.3, "distort": 0.4, "tremolo": 2,
    "compressor": -10, "pan": 0.3, "phaserrate": 2, "transient": 0.5, "delay": 0.5,
    "delaytime": 0.2, "delayfeedback": 0.3, "room": 0.4 }
  ```

- **The probe sounds.** One representative per `SourceKind` variant: the first `NAMES` row of
  each kind, in `NAMES` order. Today that is `one`, `supersaw`, `pulse`, `bus`, `sine` and
  `white`. A new variant gets a probe sound with no edit. The research probe used `sawtooth` for
  `Osc`. `NodeTable::plan(params, kind)` (`chain.rs:791`) branches on the kind, not the name, so
  `sine` plans the same table. The pin test checks it.
- **Two voices per sound.** For each probe sound there are two voices:
  - **(a)** the FX map with that `s`, plus `djf` 0.3, `vib` 2, `vibmod` 0.5, `pwsweep` 0.3 and
    `fmi` through `fmi8` at 1;
  - **(b)** `{ s, FX: [FX map + dry 0.5] }`.
- **Each table** is `NodeTable::plan(&parse_voice_params(&m, 0.25, 2.0)?, kind)`, with `lfo_0`
  set to `[Lfo(0)]` and `env_0` to `[Env(1)]` on `FxKey::Main`. A map that fails to parse is
  skipped. The pin test below catches the loss.
- **Each row** `(key, _, _)` is resolved on every table, for `fxi` `"main"` and `0`. `key` splits
  at its first `_` into `head` and `sub`. The control is `"{head}_0"` for the heads `lfo`, `env`
  and `bmod`, and `head` otherwise. `subControl` is `sub` when there is one. A row is a target when
  any resolution gives `Resolution::Targets`.
- **The result** is collected into a `BTreeSet<&'static str>`, then into a `Vec`. That is byte
  order, the order `MODULATOR_TARGETS` holds today (its sortedness test uses `<` on `&str`).

`controls_json` prints `allowed_sounds()` and `modulatable_targets()` in place of the two consts.
Its fields, their order and every other list stay as they are. `MODULATOR_TARGETS` is deleted.

The probe runs only when the dump is printed (12 tables, 118 rows, two `fxi`). It is never on a
render path.

### Tests: replace, don't layer

**Stage 001 moves tests with their code.**

- **`capability`'s tests** are the ones that assert detection, the lists or the dump:
  - `a_non_scalar_distorttype_is_unsupported`, `unsupported_signature_rules`,
    `signatures_are_distinct_with_first_bar` and `job_with`;
  - `allowlists_are_sorted_and_unique` and `chain_and_modulator_tables`;
  - the audit helpers (`AuditRow`, `is_task_ref`, `audit_rows`, `refused_sound_rows`) and their
    tests (`refused_sounds_are_not_allowed`, `only_sample_and_user_dsp_keys_stay_unsupported`,
    `the_audit_table_matches_the_allowlist`);
  - `table_job`, `shared_signature_table` and `controls_json_shape`.
- **`bus_controls` splits.**
  - The `params(...)` assertions stay in `params` as `bus_controls`.
  - The rest moves to `capability` as `bus_signatures`, unchanged: the `ALLOWED_SOUNDS` and
    `ALLOWED_KEYS` checks, the `signature_of` assertion and the two `signature_with`
    assertions.
- **`coerce` takes** `js_number_coercion` and `value_coercions`.
- **Everything else** goes to `params`, including `fm_control_names_follow_the_matrix_rule`. That
  test imports `capability::ALLOWED_KEYS`.
- **The helpers `map`, `params` and `close`** go with the tests that call them. A helper used on
  both sides is copied; each is 2 to 4 lines.

**Stage 002 replaces these:**

- **The length pin** (`assert_eq!(CHAIN_KEYS.len(), 87)`) gives way to
  `chain_keys_are_what_an_entry_reads`, a two-way read test. It needs a base map:

  ```json
  { "cutoff": 800, "hcutoff": 300, "bandf": 900, "lpenv": 1, "hpenv": 1, "bpenv": 1,
    "lprate": 1, "hprate": 1, "bprate": 1, "compressor": -10, "phaserrate": 2,
    "phaserdepth": 0.5, "tremolo": 2, "delay": 0.5, "delaytime": 0.2, "delayfeedback": 0.3,
    "room": 0.4 }
  ```

  The test uses that base and the same base without `delaytime`.
  - Key `k` **is read** when some probe value (`0.37`, `"ladder"`, `2`, `null`) on either base
    makes `parse_entry_params(base + {k: x}, 1.0)` differ from `parse_entry_params(base - k, 1.0)`.
  - For every `k` in `ALLOWED_KEYS`, being read equals `CHAIN_KEYS.contains(k) && k != "i"`.
  - `i` is asserted to be in `CHAIN_KEYS` and not read, with a comment: superdough reads it per
    entry (superdough.mjs:628, the `ignored` row in `controls.md`); the engine ignores it.
  - **A throwaway run comes first.** The research covered `CHAIN_KEYS` and the non-chain keys
    without `null`, not this exact test. Before committing, run the final test as written:
    both bases, all four probe values (`null` included) and every `ALLOWED_KEYS` key. Record its
    result in the stage report. A mismatch is reported, never fixed by editing `CHAIN_KEYS`.
- **The `MODULATOR_TARGETS` assertions** move to `modulate`'s tests, against
  `modulatable_targets()`:
  - sorted and unique;
  - `get_control_data(t, "undefined").is_some()` for every target;
  - the exclusion list (`bmod`, `dry`, `busgain`, `stretch`, `wtrate`, `tremolophase`, `lprate`,
    `lpsync`, `wt`, `transient`) is not in it.
- **`ALLOWED_SOUNDS` becomes `allowed_sounds()`** in `allowlists_are_sorted_and_unique`,
  `refused_sounds_are_not_allowed`, `bus_signatures`, `controls_json_shape`, `tests/bin.rs` and
  `tests/never_panics.rs`. Each call site changes and its assertions stay.

**Stage 002 adds these:**

- **`modulatable_targets_are_todays`** (modulate) compares against a committed list of today's 91
  names, held in the test (`TODAY`).
  - **Why a list in the test, not `controls.md`:** `controls.md` classifies keys. It has no
    target listing, and being a target is a property of a `CONTROL_TARGETS` row, not of a key's
    class. `note`, `s` and `freq` are targets through `frequency`, for example. Adding a target
    column to the audit is the same trap as the rejected chain column below.
  - **What the pin catches:** the derivation depends on probe voices. A pin catches the probe
    shrinking (a `NodeTable::plan` change that stops exposing a node on them) and growth alike.
    Changing the pin is a deliberate edit, in the same diff as the dump change it causes.
- **`every_target_row_is_derived_or_never_unsupported`** (modulate) checks that no `TARGETS` row
  left out of `modulatable_targets()` resolves to `Resolution::Unsupported` on any probe table.
  Every such row only skips or throws. A row that resolved to `Unsupported` and never to
  `Targets` would be a target the engine refuses, and the dump would hide it.
- **`source_names_match_the_builders`** (chain) checks:
  - `NAMES` names are unique;
  - a row is `Noise` exactly when `NoiseKind::from_name` is `Some`;
  - every `NoiseKind` appears as a `Noise` row. The test uses an exhaustive `match` over the
    variants, so a new variant fails to compile until it is listed;
  - `SourceKind::of(DEFAULT_SOUND) == Some(SourceKind::Osc)`.
- **`every_allowed_sound_renders`** (capability) renders one job per sound in
  `allowed_sounds()`, with every value's hap in one pause at time 0. The job has the shape of
  the guard's extra jobs: 44100 Hz, 22050 frames, seed 0, cps 1, duration 0.25. So the stage
  001 findings (below) are what the test sees.
  - **Sounds that build** (not in `SILENCE`) render with no errors, finite samples and a nonzero
    peak.
  - **`crackle`** keeps the nonzero-peak assertion, at seed 0. It is sparse by design: a hit
    has probability `density * 0.01` = 0.0002 per sample (`DEFAULT_DENSITY`, `noise.rs:33`; the
    draw, `:117`). A dry run of the guard's crackle job gave 8 nonzero samples across both
    channels, peak 0.157. Seed 5 renders silent over the same frames. The seed is
    deterministic, so the assertion is stable. A comment says why the seed is pinned. If seed 0
    renders silent, that is a finding to report. The seed is not changed and the assertion is
    not weakened.
  - **`bus`** needs a writer: values `[{ "s": "sine", "bus": 1, "dry": 0 }, { "s": "bus", "n": 1 }]`.
    The test also asserts that the writer alone (`[{ "s": "sine", "bus": 1, "dry": 0 }]`) is
    all zeros, so the pair's peak can only come through the bus. The dry run agrees: the pair
    sounds, while the writer alone and `bus` alone are silent.
  - **Silence markers** render all zeros with no errors.
  - A sound that renders silent by design is a finding to report. It is never dropped from the
    test.
- **`a_silence_string_in_an_array_stays_accepted`** (capability) asserts that `signature_of` of
  `{"s":["-"]}`, `{"s":["~"]}`, `{"s":["_"]}` and `{"s":["sine"]}` is `None`. This pins the
  re-verification's caveat.

**Not counted as coverage.** The sorted and unique checks on the derived lists mostly hold by
construction. `modulatable_targets()` comes out of a `BTreeSet`, and `allowed_sounds()` sorts.
The sounds are unique when the `NAMES` names are, which `source_names_match_the_builders`
checks, and `SILENCE` shares no name with `NAMES`. Those checks stay, but they prove little.
The real guards are three tests: the `TODAY` pin, `source_names_match_the_builders` and
`every_allowed_sound_renders`.

**Unchanged and green:** the shared signature table (`tests/data/unsupported-signatures.json`,
read by `capability` and `tests/bin.rs`), `fixtures.rs` (`KEPT_UNSUPPORTED`), `micro.rs`,
`never_panics.rs`'s coverage, and the `controls.md` audit tests. Only their import paths and the
`allowed_sounds()` call sites change.

### Docs

- **AGENTS.md (`:212-213`)** replaces "`controls` (the allowlist, audited in `controls.md`) and
  `signatures` (the binary's unsupported detection, called by `scripts/render/rust-engine.ts`)"
  with: "`capability` (what the engine plays: the key allowlist audited in `controls.md`, the
  sounds, chain and modulator keys, the unsupported detection behind the binary's `signatures`
  subcommand, called by `scripts/render/rust-engine.ts`, and the `controls --json` dump),
  `params` (the typed controls a voice reads, superdough's defaults and its mutation of a hap
  value), `coerce` (JS `Number`, `String`, truthiness and `Math.max`/`min`)". This is stage 001;
  AGENTS.md:25 keeps its `signatures` subcommand row.
- **Stage 001 also fixes every path that stops existing:**
  - README `:487` and `controls.md:41`: `src/controls.rs` becomes `src/capability.rs`;
  - README `:507`: `controls::signature_of_with_bus` becomes `capability::signature_of_with_bus`;
  - `js.rs:320`: `controls::prepare_value` becomes `params::prepare_value`;
  - `tests/bin.rs:41`: `src/controls.rs` becomes `src/capability.rs`.
- **Stage 002 updates the text about the derived lists:**
  - `controls.md:42`: "Sounds are `allowed_sounds()`: the sources the engine builds
    (`SourceKind::NAMES`) and the silence markers";
  - README `:21`: `controls --json` prints the allowlist, and its `sounds` and `modulatorTargets`
    are derived from the engine's own tables;
  - README `:487`: `CHAIN_KEYS` is checked two ways against what an FX entry reads.

## What must not change

These are the re-verification's "What must not change", restated in every stage. Each one
names the guard that proves it, and claims no more than that guard covers.

- **The `signatures` output** (`[{s, keys, firstBar}]`). The equality guard covers its 226 jobs
  (the 181 refs and 45 extra jobs). The shared signature table
  (`tests/data/unsupported-signatures.json`, committed expectations) pins the rules through
  `capability`'s `shared_signature_table` and `tests/bin.rs`. No guard covers jobs outside
  those sets.
- **The `controls --json` dump**, its fields, their order and every value. The guard compares
  it byte for byte, and `tests/bin.rs` (`controls_json_prints_the_allowlist`) checks it too.
- **The engine binary's render output, error texts and exit codes.** The guard covers its jobs
  through the WAV sha256, stdout without `renderMs`, the stderr sha256 and the exit code.
- **The CLI's refusal and what `render:check` reports as `unsupported`.** Both are built from
  `unsupported_signatures`, guarded above. `strudeland-render`'s `plan.rs` changes only its
  import, and its refusal tests run in the gates. The `fixture-*` jobs in `tests/refs` are the
  chunk jobs `render:check` plans for the corpus fixtures that have refs
  (`scripts/engine/refs.ts`). The guard does not run `render:check` itself.
- **Live warnings.** The guard does not cover them: it runs only the offline binary. They are
  covered by `live.rs`'s unit tests (`unsupported_values_are_skipped_with_one_warning_per_signature`,
  `a_bus_cycle_is_unsupported`, `a_later_value_that_closes_a_bus_cycle_makes_earlier_readers_unsupported`)
  and by `tests/live_equals_offline.rs`, which asserts that no hap is skipped on its supported
  fixtures. `signature_of_with_bus` changes module, but its body does not.
- **`{"s":["-"]}` stays accepted.** The guard's extra job checks it, and so does 002's
  `a_silence_string_in_an_array_stays_accepted`.
- **`never_panics`' coverage order.** `allowed_sounds()` equals today's list in order. The
  guard's dump check, `controls_json_shape` and `tests/bin.rs` compare it.
- **No DSP line changes.** 001's move diff checks this, and the guard's renders check it for
  its jobs.

## The equality guard

The tests are relative: they move when the code moves. The absolute guard is an A/B run against
a binary built from `385724a` before any edit. [001-split.md](proposal.md)
step 1 gives the script, `$S/0122-baseline/eq.sh`, with `$S` the scratchpad.

- **`baseline`** builds the release `strudeland-engine` binary. It refuses unless
  `git diff --quiet 385724a -- crates` holds. It writes the extra jobs, then records:
  - `controls --json`;
  - for every `crates/strudeland-engine/tests/refs/*.job.json` (181) and every extra job: the
    `signatures` output with its exit code, and the `render` exit code, WAV sha256, stdout
    without `renderMs` and stderr sha256.
- **The extra jobs** are one per value list, 45 in all:
  - each of today's 20 sounds, `x01`-`x21`, with `bus` twice: with a writer (`x05`) and alone
    (`x06`);
  - `{}`;
  - `s` as `["-"]`, `["~"]`, `["_"]`, `["sine"]`, `[["sine"]]`, `1`, `null`, `""`, `true`,
    `"Sine"` and `"sine "`;
  - a bank prefix on `sine` and on `-`, `{"bank":"x"}` with no `s` (refused as `x_triangle`),
    and `{"s":"sine","bank":""}` (a falsy bank, no prefix);
  - the refused `bytebeat`, `sbd`, `wt_a`, `z_sine`, `zzfx` and `piano`;
  - an FX entry with `i` and `ir`;
  - an `lfo` onto a noise source's `note`, the `Resolution::Unsupported` path (refused as
    `lfo:note`). No ref job has a modulator on a noise source; the shared signature table has
    this case ("lfo on a noise source"), but only in the unit and `tests/bin.rs` tests. An `lfo`
    with `control: "frequency"` is not this path: superdough has no control data for
    `frequency`, so it skips with a logged error. The dry run showed that.

  The baseline makes three checks. Every extra job's `signatures` run must exit 0. The sound
  jobs `x04`-`x21` (all but the silence markers) must render with exit 0. The dump must hold 20
  sounds and 91 targets. It then reports three findings for the stage 001 report: whether the
  bus pair (`x05`), `bus` alone (`x06`) and `crackle` (`x07`) are silent or sounding. It does
  this by comparing each WAV hash with the all-zero render of `-` (`x01`). The dry run gave
  sounding, silent and sounding.
- **`check`** rebuilds from the working tree, records again and runs `diff -r` against the
  baseline. It exits 0 only when nothing differs.

Nothing under `$S` is committed. The script is copied to `~/.cache/strudeland-0122/`, so the guard
survives a cleared scratchpad. 001 gives the recovery steps.

## Rejected

- **A `controls` re-export facade.** It would keep two paths to every item, and nothing would
  stop new code using the old one. The move is mechanical, and the compiler finds every site.
- **Deriving `ALLOWED_KEYS`.** It is the audit record. `controls.md` and its two-way test are
  the reason it can be trusted. Deriving it from the parsers would make the audit check itself.
- **A chain column in `controls.md`.** The two-way chain-read test checks `CHAIN_KEYS` against
  the code that reads FX entries, which is the only thing a column could be checked against.
- **Dropping `modulatorTargets` from the dump.** That would change the dump. It is documentation,
  but the dump's shape is part of what must not change.
- **Dropping the `SILENCE` guard from the sound check.** That would refuse `{"s":["-"]}`, which
  is accepted today.
- **A `LazyLock` static named `ALLOWED_SOUNDS`.** It would keep the old name, but it hides a
  derivation behind a static whose type compares awkwardly with `Vec<&str>`. A function says what
  it is.
- **Putting the coercions in `js.rs`.** `js.rs` is the set of ports checked bit for bit against JS
  vectors. The coercions are checked by unit tests and have no vectors.
- **Fixing the `schedule.ts` comment now.** It is a planner file, so the edit would force a refs
  regeneration and a host rebuild for a comment.
- **Replacing the `user` fallback literal with `DEFAULT_SOUND`.** It is a different rule
  (synth.mjs's `user` with no partials). The two only happen to share a value.

## Dependencies

- After #121 (merged as `385724a`).
- **Not needed:** `engine:refs`, `engine:vectors`, `render:vectors`, `render:check`, goldens,
  `host:build`, `host:parity`, `ipc:build` and `bun test`. No TS, planner or `host.inputs.json`
  file is touched, and no engine output changes.

## Risks

- **Dead code or `private_interfaces` after narrowing.** The scratch copy passed clippy with
  `-D warnings`, but stage 001 re-runs clippy on the real split. If a lint fires anyway, it is
  fixed in three ways:
  - an item only tests use gets `#[cfg(test)]`;
  - an item nothing uses is deleted, and the stage reports it;
  - a lint is never silenced with `#[allow(dead_code)]`.
- **A probe that shrinks or grows.** `modulatable_targets_are_todays` and the guard's dump check
  catch both.
- **The chain-read test's probe values.** The research run used no `null` for the non-chain
  sweep. So 002 starts with a throwaway run of the exact final test and records the result in
  the stage report before committing. If the two-way test disagrees with `CHAIN_KEYS`, that is
  a finding to report. `CHAIN_KEYS` is never edited, and the probe never narrowed, to make the
  test pass.
- **Order.** `allowed_sounds()` and `modulatable_targets()` sort in byte order, which is today's
  order. The guard compares the dump byte for byte, and `never_panics` indexes the same list.
- **The scratchpad can be cleared.** The baseline needs only `385724a`, which is
  `origin/main`. Recover it from `~/.cache/strudeland-0122/eq.sh` as 001 describes. Never use a
  scratch worktree.
- **A stale local `main`.** The local `main` branch can lag `origin/main` (while planning it
  was `77e210f`). So every diff and check names `385724a`, never `main`.
- **`cargo test -p strudeland-render` needs `cmake` and a C compiler**, and
  `cargo test --workspace` needs the V8 download. The first build takes minutes.

## Complexity

Small to medium. About 25 files change, all in one crate plus one import in
`strudeland-render`, and no behaviour changes.

- **engine:**
  - `capability.rs`, `params.rs` and `coerce.rs` are new, and `controls.rs` is deleted;
  - `lib.rs`, `chain.rs`, `voice.rs`, `modulate.rs`, `live.rs`, `render.rs` and
    `render_core.rs` change;
  - the coercion users (`adsr`, `bus`, `delay`, `job`, `js`, `orbit`, `periodic`, `reverb`,
    `stems` and `vowel`) change their imports;
  - so do the binary and `tests/{bin,fixtures,micro,never_panics}.rs`;
- **render:** `plan.rs` (one import);
- **docs:** AGENTS.md, the engine README and `controls.md`.

There are two sequential stages, each one commit, each green on its own. The plan is one
document, so there are no fracture or loss-check stages.

## Decisions

1. **Three modules: `capability` (public), `params` and `coerce` (crate-only).** No facade.
2. **`SILENCE` and `is_silence` live in `params`,** beside `prepare_value`, the mutation that
   drops a silent hap. `is_silence` is `pub(crate)`. So the silence rule is in one place, and
   the dependency runs one way, from `capability` to `params`. `capability` reads
   `params::SILENCE` for `allowed_sounds()` and the sound check, and `params::is_silence` in
   `signature_with`. Nothing outside the crate reads `SILENCE`.
3. **`allowed_sounds()` is a public function,** re-exported at the crate root in place of
   `ALLOWED_SOUNDS`.
4. **`DEFAULT_SOUND` and `SourceKind::NAMES` live in `chain.rs`,** beside `SourceKind`: the
   default is the source a value without `s` builds. `OSC_SOUNDS` is deleted.
5. **`modulatable_targets()` lives in `modulate.rs`** beside `TARGETS` and `resolve`, as
   `pub(crate)`. Its probe tables are production code, because the dump calls it.
6. **The target pin is a list in the test, not in `controls.md`** (see Tests).
7. **Stage 001 fixes every stale path,** so each commit's docs are true. Stage 002 changes only
   the text about what is derived.
8. **No GLOSSARY term.** "Capability" is an engine module name, and the engine README and
   AGENTS.md describe it.
9. **The guard covers renders too.** Stage 002 touches `SourceKind::of` and the default sound,
   which every voice build reads. The 181 refs renders cost minutes and prove no sound changed.
