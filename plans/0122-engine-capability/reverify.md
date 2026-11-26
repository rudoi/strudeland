# 0122 step 0: re-verification at 385724a (origin/main)

The issue stands. `crates/strudeland-engine/src/controls.rs` has 3252 lines: code up to 1860, tests after.

| What | Where (controls.rs unless noted) |
|---|---|
| `ALLOWED_KEYS` (420) | :14-435 |
| `ALLOWED_SOUNDS` (20) | :438-441 |
| `SILENCE` | :444 |
| `CHAIN_KEYS` (87) | :451-539, pinned by `assert_eq!(CHAIN_KEYS.len(), 87)` at :2713 |
| `CHAIN_UNSUPPORTED_KEYS` | :542-549 |
| `MODULATOR_KEYS` | :554-558, already derived from `modulate::{BMOD,ENV,LFO}_KEYS` |
| `MODULATOR_TARGETS` (91) | :564-656; only `controls_json` reads it, and it mirrors `modulate::TARGETS` (modulate.rs:51, 118 rows) by hand |
| JS coercions | :665-835 |
| typed params and superdough's mutation (`sound_of`, `is_silence`, `prepare_value`) | :837-1683 |
| unsupported detection | :1685-1837 |
| JSON dump (`controls_json`) | :1839-1859 |

- **The sound list is checked against nothing.** `SourceKind::of` (chain.rs:600-614), `OSC_SOUNDS` (:596-598) and `NoiseKind::from_name` (noise.rs:49) are never compared with `ALLOWED_SOUNDS`. The `'triangle'` default is written twice: controls.rs:1656 and voice.rs:2258.
- **AGENTS.md names a module that does not exist.** AGENTS.md:212-213 lists a `signatures` module; the detection lives in controls.rs.
- **Consumers:**
  - the binary's `signatures` and `controls --json` subcommands (`bin/strudeland-engine.rs`);
  - `render.rs:97`;
  - `live.rs:283` (the live warning);
  - `modulate.rs:631`;
  - `strudeland-render/src/plan.rs` (the refusal);
  - `lib.rs:43` re-exports;
  - `scripts/render/rust-engine.ts`.
- **Tests that read the lists:**
  - the shared signature table (`tests/data/unsupported-signatures.json`, read by controls.rs and `tests/bin.rs`);
  - `fixtures.rs` (`KEPT_UNSUPPORTED`);
  - `never_panics.rs`, which indexes `ALLOWED_SOUNDS` by position, so the order matters;
  - the `controls.md` audit tests.

## The derived sets equal today's (checked by probe)

- **Sounds.** The names `SourceKind::of` accepts (4, plus 9 `OSC_SOUNDS`, plus 4 noise kinds) and `SILENCE` (3) make exactly today's 20, in the same sorted order. One caveat: a non-string `s` whose JS string form is a silence marker (`{"s":["-"]}`) is accepted today, and its hap then fails with "sound - not found". So the check must stay `SourceKind::of(s).is_some() || SILENCE.contains(s)`; dropping the `SILENCE` half would refuse it.
- **Modulator targets.** Running `modulate::resolve` over every `TARGETS` row against probe node tables gives exactly the 91 listed targets. The 27 rows left out always skip or throw.
- **Chain keys.** Every `CHAIN_KEYS` entry changes `parse_entry_params` except `i` (read by superdough, ignored by the engine) and `delaysync` (it counts only when `delaytime` is absent). No allowed key outside `CHAIN_KEYS` changes it. So a two-way read test can replace the length pin.

## What must not change

- the `signatures` output (`[{s, keys, firstBar}]`);
- the `controls --json` dump: its fields, their order and its values;
- render refusal texts and exit codes;
- what `render:check` reports as `unsupported`;
- live warnings;
- what the engine accepts or refuses;
- `never_panics` coverage order.

Nothing in the DSP changes. `scripts/engine/provenance.ts` hashes no Rust file, so engine refs and vectors are not regenerated.
