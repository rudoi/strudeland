# 002: derive the sound list and the modulator targets from the engine's own tables

## Goal

Two hand-kept copies give way to derivations from the code that decides:

- `ALLOWED_SOUNDS` gives way to `capability::allowed_sounds()`, from `SourceKind::NAMES` and
  `SILENCE`;
- `MODULATOR_TARGETS` gives way to `modulate::modulatable_targets()`, from `TARGETS` through
  `resolve`.

**One default.** `DEFAULT_SOUND` serves both `params::sound_of` and `voice::sound_name`.

**The remaining hand lists get two-way tests:**

- `CHAIN_KEYS` against what `parse_entry_params` reads;
- the derived targets against a committed pin;
- every accepted sound against a real render.

The docs say what is derived.

**No output changes.** The equality guard must still say `equal`. That covers the dump byte for
byte (`sounds` and `modulatorTargets` included, in today's order), every `signatures` output and
every render.

## Prerequisites

- **Stage 001 is committed,** and its gates are green.
- **`$S/0122-baseline/base/` exists.** If not, recover it as [001-split.md](001-split.md) step 1
  says before editing anything.
- **Read the proposal's "Derivations" and "Tests" sections.** They give the exact probe maps,
  values and assertions.

## Implementation

### Step 1: `SourceKind::NAMES` and `DEFAULT_SOUND` (`chain.rs`)

- **Add `pub const DEFAULT_SOUND: &str = "triangle";`** above `SourceKind`, with the doc "The
  sound a value without `s` plays (superdough's `s = 'triangle'` default)".
- **Add `SourceKind::NAMES: &'static [(&'static str, SourceKind)]`.** It has 17 rows, in today's
  match order:
  - `("one", Constant)`, `("supersaw", Supersaw)`, `("pulse", Pulse)`, `("bus", Bus)`;
  - `sine`, `sawtooth`, `square`, `triangle`, `sin`, `saw`, `sqr`, `tri` and `user`, each `Osc`;
  - `white`, `pink`, `brown` and `crackle`, each `Noise`.
- **Rewrite `SourceKind::of`** as a lookup in `NAMES` (first match). Its doc and signature stay.
- **Delete `OSC_SOUNDS`.** `of` was its only reader; `git grep -n OSC_SOUNDS` must print nothing.
- **Leave alone:**
  - `NoiseKind::from_name`, which `fmwave` reads;
  - `canonical`, `WaveKind::builtin`;
  - the `user` fallback literal in `build_oscillator` (`voice.rs:1103-1107`).
- **Test `source_names_match_the_builders`** in `chain.rs`'s tests checks:
  - the names are unique;
  - for every row, `kind == Noise` exactly when `NoiseKind::from_name(name).is_some()`;
  - every `NoiseKind` appears as a `Noise` row. Use an exhaustive `match` over
    `White | Pink | Brown | Crackle`, so a new variant fails to compile until it is listed;
  - `SourceKind::of(DEFAULT_SOUND) == Some(SourceKind::Osc)`.

### Step 2: one default

- **`params::sound_of`:** `.unwrap_or_else(|| Value::Str(DEFAULT_SOUND.into()))`.
- **`voice::sound_name`:** `s.map_or_else(|| DEFAULT_SOUND.to_string(), js_to_string)`.
- **Docs:** both doc comments say "with the [`DEFAULT_SOUND`] default". The `VoiceParams::s`
  field doc (`"triangle"` when absent) names `DEFAULT_SOUND` too.
- **Check:** `voice.rs`'s `assert_eq!(sound_name(None), "triangle")` stays as it is. It is the
  pin on the value.

### Step 3: `allowed_sounds()` and the sound check (`capability.rs`)

- **Add the function:**

  ```rust
  /// Every `s` the engine accepts, sorted by byte order: the sounds it builds
  /// ([`SourceKind::NAMES`](crate::chain::SourceKind)) and the silence markers ([`SILENCE`]).
  pub fn allowed_sounds() -> Vec<&'static str>
  ```

  It chains the `NAMES` names with `SILENCE`, then calls `sort_unstable()`. Do not dedup: the
  test asserts uniqueness.
- **Delete `ALLOWED_SOUNDS`.**
- **In `signature_with`,** replace `ALLOWED_SOUNDS.binary_search(&s.as_str()).is_ok()` with
  `SourceKind::of(&s).is_some() || SILENCE.contains(&s.as_str())`. `s` is the same bank-prefixed
  JS string as today. Add a comment: "the silence half keeps a non-string `s` whose JS string is a
  marker (`["-"]`) accepted; `is_silence` sees only strings".
- **`lib.rs`:** `pub use capability::{ALLOWED_KEYS, allowed_sounds, controls_json};`.

### Step 4: `modulatable_targets()` (`modulate.rs`)

- **Add the probe.** `fn probe_tables() -> Vec<NodeTable>` (private) builds the 12 tables, and
  `pub(crate) fn modulatable_targets() -> Vec<&'static str>` resolves every `TARGETS` row on them.
  Both follow the proposal's "Derivations" exactly:
  - the FX map;
  - the six sounds, each as voice (a) and voice (b);
  - `parse_voice_params(&m, 0.25, 2.0)` and `NodeTable::plan`;
  - `lfo_0` and `env_0` registered on `FxKey::Main`;
  - the control and `subControl` from the row key;
  - `fxi` `"main"` and `Value::Num(0.0)`;
  - any `Resolution::Targets` counts;
  - a `BTreeSet` into a `Vec`.

  Build the maps with `serde_json::json!` and `serde_json::from_value::<ValueMap>`, and skip a
  map that fails to deserialize or parse. Never `unwrap` in this production path.
- **Share the classifier.** Factor the per-row resolution as a private
  `fn row_outcomes(table: &NodeTable, key: &str) -> [Outcome; 2]` (one per `fxi`), so
  `modulatable_targets()` and the tests read the same classification. `Outcome` is a private enum:
  `Targets`, `Skip`, `Unsupported` or `Throws` (an `Err` from `resolve`).
- **`capability::controls_json`:** print `allowed_sounds()` in place of `ALLOWED_SOUNDS` and
  `crate::modulate::modulatable_targets()` in place of `MODULATOR_TARGETS`. Its `to_json` closure
  takes `&[&str]`, so pass `&vec`. The format string and field order stay unchanged.
- **Delete `MODULATOR_TARGETS`.** `git grep -n MODULATOR_TARGETS -- crates` must print nothing.
- **Tests in `modulate.rs`:**
  - **`modulatable_targets_are_todays`:** `assert_eq!(modulatable_targets(), TODAY)`, with
    `const TODAY: [&str; 91]` holding the 91 names from `385724a`'s `MODULATOR_TARGETS` (copy them
    from `git show 385724a:crates/strudeland-engine/src/controls.rs`, lines 564-656). The comment
    says why the pin is in the test (proposal, Tests): `controls.md` lists no targets, and the pin
    catches a probe that shrinks or grows.
  - **`modulatable_targets_are_known_controls`** moves today's `MODULATOR_TARGETS` assertions from
    `chain_and_modulator_tables`:
    - sorted and unique (`windows(2)`, `<`);
    - `get_control_data(t, "undefined").is_some()` for every target;
    - the ten excluded names are absent.
  - **`every_target_row_is_derived_or_never_unsupported`:** every `TARGETS` row is either in
    `modulatable_targets()` or gives only `Skip` and `Throws` on every probe table, never
    `Unsupported`. The failure message names the row. Today 27 rows are left out, all skip or
    throw (research probe).

### Step 5: the capability tests

- **`chain_and_modulator_tables`:**
  - delete `assert_eq!(CHAIN_KEYS.len(), 87)`;
  - delete the `MODULATOR_TARGETS` assertions, which moved in step 4;
  - keep everything else: the subset and disjointness checks, the filter families, the in list
    (`fanchor`, `ftype`, `drive`, `lfo`, `env`, `bmod`, `dry`, `room`, `delay`), the out list
    (`s`, `note`, `orbit`, …, `FXrelease`) and the `MODULATOR_KEYS` checks.
- **Add `chain_keys_are_what_an_entry_reads`** (proposal, Tests):
  - the base map, and the same base without `delaytime`;
  - the probe values `0.37`, `"ladder"`, `2` and `null`;
  - for every `k` in `ALLOWED_KEYS`, `read(k) == (CHAIN_KEYS.contains(&k) && k != "i")`;
  - `CHAIN_KEYS.contains(&"i") && !read("i")`, with the comment that superdough reads `i` per
    entry (superdough.mjs:628) and the engine ignores it (`controls.md`: `ignored`).

  The failure message names the key. **A mismatch is a finding: report it.** Never edit
  `CHAIN_KEYS`, never narrow the probe values, and never add an exception to make the test pass.
- **`allowlists_are_sorted_and_unique`:**
  - check `allowed_sounds()` (as a slice) in the sorted loop;
  - drop `MODULATOR_TARGETS` from it, since step 4 covers it;
  - `SILENCE ⊆ allowed_sounds()`.
- **`refused_sounds_are_not_allowed`:** use `let allowed = allowed_sounds();` and read it where
  the test read `ALLOWED_SOUNDS`.
- **`bus_signatures`:** `assert!(allowed_sounds().contains(&"bus"))`.
- **`controls_json_shape`:**
  - `v["sounds"]` equals `json!(allowed_sounds())`. That is stricter than the length it checked;
  - `v["modulatorTargets"]` equals `json!(crate::modulate::modulatable_targets())`.
- **Add `every_allowed_sound_renders`** (proposal, Tests). A local helper builds a job with every
  value's hap in one pause at time 0 (schema 2, 22050 frames, the orbit and channels of
  `table_job`). Then:
  - **For each sound not in `SILENCE`:** `crate::render(&job)` gives no errors, every sample is
    finite and the peak is above 0. `bus` uses `[{ "s": "sine", "bus": 1, "dry": 0 }, { "s":
    "bus", "n": 1 }]`.
  - **For each silence marker:** no errors, and all zeros.
- **Add `a_silence_string_in_an_array_stays_accepted`:** `signature_of` of `{"s":["-"]}`,
  `{"s":["~"]}`, `{"s":["_"]}` and `{"s":["sine"]}` (cps 0.5) is `None` for each.

### Step 6: the integration tests

- **`tests/bin.rs` (`controls_json_prints_the_allowlist`):** `assert_eq!(sounds,
  strudeland_engine::allowed_sounds())`. The rest is unchanged, including
  `targets.contains("cutoff") && !targets.contains("lprate")` and the `controls_json()`
  equality.
- **`tests/never_panics.rs`:**
  - import `allowed_sounds` in place of `ALLOWED_SOUNDS`;
  - in `every_allowed_key_renders_finite_audio`, add `let sounds = allowed_sounds();` before the
    loop and use `sounds[(i + j) % sounds.len()]`. Same order, same coverage.

### Step 7: docs

- **`controls.md:42`:** "Sounds are `allowed_sounds()`: the sources the engine builds
  (`SourceKind::NAMES` in `src/chain.rs`) and the silence markers; the refused ones are listed at
  the end."
- **The engine README:**
  - **`:21-22`:** "`controls --json` prints the control allowlist (`capability::controls_json`;
    nothing on the TypeScript side mirrors it). Its `sounds` are the sources the engine builds
    plus the silence markers, and its `modulatorTargets` are the `CONTROL_TARGETS` rows that
    resolve on some voice (`modulate::modulatable_targets`, pinned by a test)."
  - **`:487`, after the `ALLOWED_KEYS` sentence:** "`CHAIN_KEYS` is checked both ways against
    what an FX entry reads (`parse_entry_params`); `i` is the one key superdough reads per entry
    that the engine ignores."
- **AGENTS.md:** no change beyond 001. Its `capability` entry already says "the sounds".

## Validation

- **Run the gates** in [000-prompt.md](000-prompt.md) with `N=002`. Every log ends `exit 0`, and
  `0122-002-eq.log` says `equal`. The dump, every `signatures` output and every render are
  unchanged from `385724a`.
- **The new tests pass:**
  - `source_names_match_the_builders`;
  - `modulatable_targets_are_todays`, `modulatable_targets_are_known_controls` and
    `every_target_row_is_derived_or_never_unsupported`;
  - `chain_keys_are_what_an_entry_reads`;
  - `every_allowed_sound_renders`;
  - `a_silence_string_in_an_array_stays_accepted`.

  Find them by name in `0122-002-crates.log`.
- **The old names are gone:** `git grep -nE 'ALLOWED_SOUNDS|MODULATOR_TARGETS|OSC_SOUNDS' --
  ':!plans'` prints nothing. `git grep -n '"triangle"' -- crates/strudeland-engine/src` shows
  `DEFAULT_SOUND`'s definition, the `user` fallback, the wave tables and tests, but no default.
- **Nothing outside the engine moved:** `git diff --stat main -- src scripts tests vendor
  crates/strudeland-host crates/strudeland-live` is still empty.

## Commit message

```
refactor(engine): derive the sound list and modulator targets

The accepted sounds come from the table SourceKind::of reads plus the
silence markers (allowed_sounds), and the dump's modulator targets from
resolving every CONTROL_TARGETS row against probe voices
(modulatable_targets), so neither hand copy can drift from the engine.
One DEFAULT_SOUND serves sound_of and sound_name. CHAIN_KEYS is checked
both ways against what an FX entry reads instead of by its length, the
derived targets are pinned to today's list, and every accepted sound is
rendered. The sound check keeps its silence half, so {"s":["-"]} stays
accepted.

The controls --json dump, every signatures output and every render are
byte-identical.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
