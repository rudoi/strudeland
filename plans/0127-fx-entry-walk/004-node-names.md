# Stage 004: `NodeName`, typed node table keys

## Goal

The plan and the modulator loop register node table entries by a `NodeName` enum instead of
string literals and `format!`. Lookup stays by string, because `resolve`'s fallback is the
control's free text. A new test ties the names the plan registers to `modulate::TARGETS`' node
column.

The design is in the proposal's [NodeName](../0127-fx-entry-walk.md#nodename-stage-004)
section.

## Prerequisites

Stage 003 committed, its A/B log at `0 differences`.

## Files

- `crates/strudeland-engine/src/chain.rs`: `NodeName`, `NodeTable`'s entries and its `set`,
  `one` and `push_element`, `plan`, `plan_fm`, `plan_entry`, and the tests.
- `crates/strudeland-engine/src/modulate.rs`: `register`, `probe_tables`, the imports, and the
  tests (the cross-check, plus any `table.set` with a string).
- `crates/strudeland-engine/src/voice.rs`: only the tests, if any call `NodeTable::set` with a
  string.
- `crates/strudeland-engine/README.md`: one clause.

## Steps

1. **`NodeName`** (chain.rs, before `NodeTable`), as the proposal gives it:
   - `const FM_NAMES: [&str; 8] = ["fm_1", …, "fm_8"]` and
     `const FM_GAIN_NAMES: [&str; 8] = ["fm_1_gain", …, "fm_8_gain"]`;
   - `pub fn as_str(&self) -> &str`. `Fm(n)` is `FM_NAMES[usize::from(n) - 1]`, and the same
     for `FmGain`;
   - `pub fn filter(kind: FilterKind) -> (NodeName, NodeName)`: `Lowpass` is
     `(Lpf, LpfLfo)`, `Highpass` is `(Hpf, HpfLfo)`, `Bandpass` is `(Bpf, BpfLfo)`, and `Notch`
     is `unreachable!("the phaser is not a filter slot")`;
   - a doc comment on each variant naming its key, where the name is not obvious.
2. **`NodeTable`.**
   - `entries: Vec<(FxKey, NodeName, NodeList)>`.
   - `set(&mut self, key, name: NodeName, list)`, `one(…, name: NodeName, …)` and
     `push_element(…, name: NodeName, …)`.
   - **Find by `as_str()`:** `.find(|(k, n, _)| *k == key && n.as_str() == name.as_str())`.
     Replace in place, or push. These are today's semantics.
   - `get(&self, key, name: &str)` compares `n.as_str() == name`. `keys()` maps
     `n.as_str()`. Both keep their signatures.
3. **The plan.** Replace every literal with its variant:
   - `"source"` → `Source`; `"pw_lfo"` → `PwLfo`; `"vib"`/`"vib_gain"` → `Vib`/`VibGain`;
   - `"post"`, `"delay"`, `"delay_mix"`, `"room"`, `"room_mix"`, `"djf"` → `Post`, `Delay`,
     `DelayMix`, `Room`, `RoomMix`, `Djf` (the same variants serve the main entry's sends and a
     chain entry's local effects);
   - `"transient"`, `"gain"`, `"vowel"`, `"coarse"`, `"crush"`, `"shape"`, `"distort"`,
     `"tremolo"`, `"tremolo_gain"`, `"compressor"`, `"pan"`, `"phaser"`, `"phaser_lfo"` → their
     variants;
   - `plan_entry`'s filter loop: `let (name, lfo_name) = NodeName::filter(kind);`. Delete the
     `format!("{name}_lfo")`. The `filters` array loses its string column (stage 002 gave it
     the `FilterKind`);
   - `plan_fm`: `NodeName::Fm(idx as u8)` and `NodeName::FmGain(idx as u8)`, `idx` in 1..=8.
     Delete both `format!`s;
   - the shaper array's name column becomes `NodeName::Coarse` and so on.
4. **`modulate`.** `register` writes `NodeName::Mod(format!("lfo_{id}"))` and
   `NodeName::Mod(format!("env_{id}"))`. `probe_tables` writes `NodeName::Mod("lfo_0".into())`
   and `NodeName::Mod("env_0".into())`. Nothing else changes: `resolve`, `node_param`, `TARGETS`
   and `unsupported_targets` keep their string lookups.
5. **README.** In the sentence stage 002 wrote, after "superdough's `nodes` tracker", add
   "(its keys a `NodeName`, looked up by text as superdough does)".

## Tests

- **Added, `chain.rs`:** `node_names_are_superdough_keys`. Every static variant's `as_str()`
  equals today's literal: one row per variant, written out from the literals step 3 replaced.
  `Fm(1)`/`Fm(8)` are `"fm_1"`/`"fm_8"`, and `FmGain(1)`/`FmGain(8)` are
  `"fm_1_gain"`/`"fm_8_gain"`. `Mod("lfo_x".into())` is `"lfo_x"`. An exhaustive `match`
  helper (`fn listed(n: &NodeName)`, no wildcard arm) fails to compile on a new variant.
- **Added, `modulate.rs`:** `every_planned_name_is_a_target_node`.
  - Collect `keys(FxKey::Main)` and `keys(FxKey::Index(0))` of every `probe_tables()` table,
    minus the `lfo_`/`env_` names the probe adds.
  - Assert each one except `room` appears in `TARGETS`' node column (the middle field).
  - Assert the collected set holds every static `NodeName` text and `fm_1`…`fm_8`,
    `fm_1_gain`…`fm_8_gain`. So the probe reaches every name, and a new name with no target
    fails here.
  - The comment says why `room` is exempt: the orbit's convolver on `Main` and the local reverb
    on a chain entry, with no param a control reaches. `TARGETS` reaches the reverb's level
    through `room_mix`.

  **Check it on today's names first.** The planning research found the column holds every
  registered name except `room`. If any other name is missing, the test has found a real
  mismatch. Report it; do not add a `TARGETS` row (that would change `modulatable_targets` and
  the dump).
- **Changed access path only:** tests that call `NodeTable::set` with a string literal take the
  variant. `chain.rs`'s node table unit test,
  `node_table_set_replaces_push_appends_and_a_missing_key_is_none` (its `set` calls at
  chain.rs:1772-1846), writes
  `"lpf"`/`"lpf_lfo"`/`"gain"`/`"hpf"`. Same assertions. A test that needs a name the enum does
  not have uses `NodeName::Mod`, with a comment.
- **Unchanged:** every `get(…, "name")` and `keys(…)` assertion, and every modulate
  "Available targets" assertion.

## Gates

[000-prompt.md](000-prompt.md) "Test cadence", with `N=004`. The guard jobs for this stage are
the "Available targets" jobs `y01`-`y07`, the refs jobs with modulators, `x45` and `y34` (the
refused targets) and the dump (`modulatorTargets`).

## STOP conditions

- Any A/B difference, an "Available targets" list or the dump above all.
- `every_planned_name_is_a_target_node` fails on a name other than `room`.
- Anything outside the plan, `register` and `probe_tables` needs a `NodeName`: `resolve`'s
  lookup must stay by string.

## Commit message

```
refactor(engine): name node table entries with a NodeName enum

The plan registered superdough's node keys as string literals and built
the FM and filter LFO names with format!. A NodeName enum names them now,
matched and looked up by the same text, so the table's order and
last-writer rule are unchanged; a test ties every planned name to a
modulator target row.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
