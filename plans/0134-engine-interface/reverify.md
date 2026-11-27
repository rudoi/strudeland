# #134 re-verification (Step 0)

Checked on `llm/andrew/strudeland/0134-engine-interface` at `7c6e632` (origin/main). The claim
was written against `63b945d` (v0.1.2). Since then: #36 `render_core`, #37 split `controls` into
`capability`/`params`/`coerce`, #45 `decay` and `tests/decay_bound.rs`, plus live and render-crate
refactors.

## 1. Engine pub surface (`crates/strudeland-engine/src/lib.rs`)

The claim's module names are stale. `controls` is gone: its 41 items now sit mostly in the
`pub(crate) mod params` and `coerce`. `js_to_number`, `VoiceParams` and `fm_suffix` are no longer
public. The pub modules today are these:

| module | lib.rs | pub items | external users | needed outside |
|---|---|---|---|---|
| `capability` | :8 | 10: `ALLOWED_KEYS` :15, `allowed_sounds` :440, `CHAIN_KEYS` :455, `CHAIN_UNSUPPORTED_KEYS` :546, `MODULATOR_KEYS` :558, `UnsupportedSignature` :570, `signature_of` :578, `signature_of_with_bus` :585, `unsupported_signatures` :683, `controls_json` :724 | `unsupported_signatures`: engine bin :20, `tests/fixtures.rs:19`, `tests/micro.rs:21`, render `src/plan.rs:7`, render `tests/decay_bound.rs:14` (new, #45). `UnsupportedSignature`: render `plan.rs:7`. `CHAIN_KEYS`: `tests/bin.rs:432`, `tests/never_panics.rs:1262`. `ALLOWED_KEYS`/`allowed_sounds`/`controls_json` already reached through the root re-export (lib.rs:60). | `unsupported_signatures`, `UnsupportedSignature`, `CHAIN_KEYS` (tests only). `CHAIN_UNSUPPORTED_KEYS`, `MODULATOR_KEYS` and `signature_of_with_bus` are used only inside the crate. `signature_of` is used only by capability's own unit tests (:754, :799, :1122, :1346). |
| `orbit` | :23 | 13 top-level: `GainPoint` :75, `DuckCurve` :84, `gain_at` :92, `DuckControls` :111, `Input` :169, `sum_channel` :176, `sum_stereo` :197, `route` :216, `orbit_pair` :233, `OrbitInputs` :244, `Orbit` :285, `Return` :313, `OrbitBank` :643 | only `tests/orbit_pairs.rs:5` (`orbit_pair`, `route`). The claim holds. | none |
| `live` | :21 | `LiveOptions`, `LiveHap`, `LiveEvent`, `LiveCounters`, `IrRequest`, `IrBuilt`, `build_ir`, `LiveEngine` | strudeland-live `player.rs:16`, `ir_worker.rs:6`, `tests/player.rs:11`; engine `tests/live_equals_offline.rs:8` | all of them |
| `jsmath` | :17 | `IMPLEMENTATION` and the math fns | host `src/natives.rs:12`, host `tests/natives.rs:7`, engine `tests/v8_math.rs:24` | all of them |
| `wav` | :43 | `WavError`, `data_size`, `sample_data_size`, `write_f32`, `DecodedWav`, `read_f32` | engine bin :21; render `encode/wav.rs:7,30`, `stems.rs:9`, `tests/{bin,pipeline,stems,encode}.rs` | all of them |
| `decay` | :52-58 | re-exports only | render `tests/decay_bound.rs:15` | yes |

Root re-exports (lib.rs:60-68) have no external user in two cases: `Engine` (`render::Engine`, lib.rs:66) and `quantum_end_time` (lib.rs:81). Everything else is used: `EngineJob`, `ErrorKind`, `JobError`, `OrbitSpec`, `RenderError`, `Value`, `Mulberry32`, `number_to_string` (doc-hidden), `DISTORTION_NAMES`/`distortion_algorithm` (`v8_math.rs`), the stems types, `Rendered`, `render`, and the constants.

src-tauri and strudeland-cli do not depend on the engine directly (Cargo dependents: host,
render, live). The engine has no benches.

## 2. Unsupported-material shapes today

There are four shapes, not three. The render crate has its own `Unsupported`.

| # | shape | defined | produced | consumed | text, where pinned |
|---|---|---|---|---|---|
| A | `capability::UnsupportedSignature { s, keys, first_bar }`, serialized `{s,keys,firstBar}` | capability.rs:567-574 | `unsupported_signatures` :683, `unsupported_signatures_in` :689 | engine bin `signatures` (bin :92-101, one JSON line). The TS `UnsupportedSignature` (`src/lib/render/engine-job.ts:101`) is parsed by `scripts/render/rust-engine.ts:417-451` and used by `rust-renderer.ts:102`, `check.ts:503,712`, `formatSignature` (engine-job.ts:401: `s=<s> keys=<k> bar=<n>`, pinned in `tests/render-report.test.ts:282-332`, `render-check-flow.test.ts:853,914`). Render crate `plan.rs:54` pre-scan, then `refuse()` :63. | JSON shape pinned by `tests/bin.rs` (signature table at :375) and `tests/data/unsupported-signatures.json` |
| B | engine `RenderError { kind: Unsupported, message: "s={s} keys={k,..}", bar: Some(first_bar) }` | job.rs:208-237 | render.rs:97-118, from A. When A is non-empty nothing plays and the output is silent. The same path serves `render_with_stems` (stems.rs:99). | engine bin `render` summary `errors` (bin :152-158), exit 1. TS `parseSummary` (rust-engine.ts:261) checks only that `kind`/`message` are strings. Render crate `render_one` re-check (render.rs:217-223). | `"s=bytebeat keys=ir"` render.rs:392, `"s=bus keys=bus-cycle"` :1650, `tests/bin.rs:221` (`{kind:"unsupported",message:"s=bytebeat keys=ir",bar:0}`), `tests/fixtures.rs:469-480` (reformats `s={s} keys={k}`), `never_panics.rs:1150` (kind only). TS fakes use the same text: `tests/render-rust-engine.test.ts:153,495,780`. With empty keys the message reads `s=zzfx keys=` (trailing `keys=`). |
| C | render crate `RenderError::Render("chunk N: unsupported: <B.message>")` | render.rs:217-223 | B, once it gets past `take_jobs` | Display `render failed: chunk N: unsupported: …` | render.rs:497-508 (fake `"s=kick"`). It cannot happen in practice: `take_jobs` refuses first (plan.rs:36-57). |
| D | render crate `Unsupported { layout_bar, s, keys, section }` inside `RenderError::Unsupported` | render error.rs:36-47 | `refuse()` plan.rs:63-88, from A | Display error.rs:82-99: `s=X[ keys=a,b] first at bar N[ (section 'x')]`, where `keys=` is left out when empty. The CLI prints it on stderr. The app's export dialog shows it through Display. | plan.rs:182,216, error.rs:203, `tests/pipeline.rs:250,303`, `tests/bin.rs:182`, cli `render.rs:1366-1373`, SKILL.md:510-512 |
| E | `LiveEvent::Warning { signature: String /* the sound s */, keys, first_cycle: f64 }` | live.rs:54-62 | live.rs:281-293 (`signature_of_with_bus`, deduped by `(s, keys)`) | strudeland-live `Event::Warning` (event.rs:30-34, tagged JSON `{"kind":"warning","signature","keys","firstCycle"}`, pinned event.rs:91-98) → `src-tauri/src/playback.rs` `Channel<Event>` → TS `playback-api.ts:19` → `rust.ts:161-163` → `EngineWarning` (backend.ts:6-10) → `runtime.ts:684-687` (deduped by `signature` only) → `engine-notices.ts:16-21`. The status bar shows `rust engine skipped <s>`, with the tooltip `keys: a, b`. | event.rs:98, `tests/engine-notices.test.ts`, `tests/runtime-backends.test.ts:402-415`, `tests/rust-backend.test.ts:477`, SKILL.md:642. Nothing reads `firstCycle`. It is not in `bindings.ts` (Channel payloads are not generated). |

## 3. Visible behaviour and wire: decisions

Each item below stays byte-identical unless a decision changes it.

1. **The `signatures` subcommand JSON (A).** The harness parses it. *Recommend: unchanged.* Only
   the Rust path changes (`strudeland_engine::unsupported_signatures`).
2. **The engine binary's `render` summary (B), exit 1 and the `unsupported` kind on the wire.**
   *Recommend: unchanged.* If `Rendered` carries `unsupported: Vec<UnsupportedSignature>`, the
   binary maps each one to `{kind:"unsupported",message:"s=… keys=…",bar}` in the summary, so
   `tests/bin.rs:221` and the TS fakes hold. Keep `ErrorKind::Unsupported` for that wire. Dropping
   it would touch `micro.rs:95,314`, `refs/thresholds.json` `engineKind` and the TS
   `RenderErrorKind`. Alternative: drop the variant and have the binary serialize the summary
   entry by hand. That is more churn for nothing visible.
3. **The surviving-unsupported failure (C).** *Recommend: keep the text*
   `render failed: chunk N: unsupported: s=<s> keys=<k>`, formatted in the render crate from
   `rendered.unsupported[0]`. The path cannot happen in practice, so the only change is the
   unit-test fixture (render.rs:502), which becomes a signature instead of a `RenderError`.
4. **The CLI and export refusal text (D).** *Recommend: unchanged and out of scope.* It is
   already structured.
5. **Live warnings (E): `firstCycle` vs bar, `signature` naming the sound, and the
   `Channel<Event>` payload.** This is a wire to the webview, but not a generated IPC binding.
   *Recommend: leave it out of scope and keep the payload identical.* At most,
   `LiveEvent::Warning` could carry an `UnsupportedSignature`-like struct inside the engine, with
   strudeland-live mapping it back to the same `{signature, keys, firstCycle}` JSON. A live hap's
   `cycle` is `hap.whole.begin` (rust.ts:320), not necessarily a layout bar, so renaming it to a
   bar is a semantic change with no consumer. If the owner wants a bar anyway, it is a separate
   decision covering event.rs:98 and the four TS files.
6. **Behaviour quirks worth noting but not fixing here:** B writes a trailing `keys=` when the
   keys are empty, and D leaves it out. The runtime dedupes live warnings by sound only, while
   the engine dedupes by `(s, keys)`. *Recommend: leave both alone.* Each one is pinned
   (render-report tests, `tests/bin.rs`), and changing them would make this a behaviour change.

## 4. Sound

No DSP change. The work is visibility plus where the refusal list lives. The refused-path audio
stays silent at `frames` length. The A/B guard is not needed. Changing the shape of `Rendered`
touches these:

- engine render.rs:124 and the tests that assert `r.errors` holds Unsupported entries:
  render.rs:388-395, :1646-1653, `tests/fixtures.rs:466-480` (assert on `rendered.unsupported`
  instead), `tests/never_panics.rs:1150`
- engine bin :152-158 (decision 2)
- render crate literals that build `Rendered`: render.rs:372 (fake), pipeline_audio.rs:211 (the
  joined result reuses the engine type) and :338 (fake)
- render crate `render_one` (render.rs:217-223) and its test :497-508
- `tests/stems.rs:33` only reads `Rendered`. Struct-literal breakage is avoided if
  `unsupported` is added with the same field visibility (the type has no `Default`, so
  `..Default::default()` is not an option; edit the literals instead).

## 5. Integration tests after `pub(crate)`

| test | uses | if `capability` and `orbit` become `pub(crate)` | cheapest fix |
|---|---|---|---|
| `tests/orbit_pairs.rs` (62 lines) | `orbit::{orbit_pair, route}` | breaks | Move it into `orbit.rs`'s `#[cfg(test)] mod tests` (:816) with `include_str!` of `tests/helpers/orbit-pairs.json`, as capability.rs:1375 already does with its table. No testing module is needed. |
| `tests/fixtures.rs`, `tests/micro.rs`, engine bin, render `plan.rs`, render `tests/decay_bound.rs` | `capability::unsupported_signatures` (+ `UnsupportedSignature`) | breaks | Root re-export `pub use capability::{UnsupportedSignature, unsupported_signatures}`, then edit 5 `use` lines. |
| `tests/bin.rs:432`, `tests/never_panics.rs:1262` | `capability::CHAIN_KEYS` | breaks | Add `CHAIN_KEYS` to the root re-export next to `ALLOWED_KEYS` (the same family; `controls --json` prints it). Alternative: `#[doc(hidden)] pub mod testing`. |
| `live_equals_offline.rs`, `stems.rs`, `v8_math.rs`, `licence.rs`, `never_panics.rs` (rest) | root items, `live`, `jsmath` | unaffected | — |

Lint risk under `clippy -D warnings`: once `capability` is `pub(crate)`, `signature_of`
(:578) is only called from `#[cfg(test)]` and gets flagged as dead code. Mark it `#[cfg(test)]`
or inline it into the tests. Check the orbit items the same way: all are used inside the crate
(`sum_stereo` in chain/modulate/stems, `route` :325, `gain_at` :359, `orbit_pair` to confirm when
the test moves). Optional: drop the root re-exports of `Engine` and `quantum_end_time` (no external
user) or make them `pub(crate)`.

## 6. New callers or copies since 63b945d

- New caller: render `tests/decay_bound.rs:14,78` (`capability::unsupported_signatures`, #45).
- New path through B: `render_with_stems` (stems.rs:99) shares `run_observing_overflow`, so stems
  renders refuse the same way.
- The `s={} keys={}` formatting is copied in engine render.rs:115, `tests/fixtures.rs:477` and TS
  `formatSignature` (engine-job.ts:401, adds `bar=`). The render crate's own variant is
  error.rs:82-99.
- Line references that moved: render.rs:447-455 → render.rs:97-118; plan.rs:56 → plan.rs:54;
  render_one :217-222 is unchanged; live.rs:55 is unchanged; `controls.rs:1691` →
  `capability.rs:570`.

## Recommendation

**Proceed, re-scoped:**

1. In `lib.rs`, `pub use capability::{UnsupportedSignature, unsupported_signatures, CHAIN_KEYS}`.
   Make `capability` and `orbit` `pub(crate)`, put `signature_of` behind `#[cfg(test)]`, and
   optionally make `Engine` and `quantum_end_time` `pub(crate)`.
2. Move `tests/orbit_pairs.rs` into the unit tests in `orbit.rs`, then update the 5 `use` lines.
3. Change `Rendered` to `{ channels, errors, unsupported: Vec<UnsupportedSignature> }`, with no
   Unsupported entries in `errors`. Both the engine binary and the render crate's `render_one`
   format from the struct, and the texts stay identical (decisions 2 and 3).
4. Out of scope: live warning payload and semantics (decision 5), the render-crate refusal text
   (decision 4), and the `keys=` and dedupe quirks (decision 6).

Gates: `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`.
`bun test` is a no-op check, because no TS changes. No refs, vectors, goldens or host rebuild are
needed: no `PLANNER_FILES` and no bundled JS are touched.

## Decisions (2026-11-27, recommended answers taken; none changes visible behaviour, the wire or sound)

1. **Unchanged output:** the `signatures` JSON stays unchanged. So do the binary's render summary entries, exit 1 and `ErrorKind::Unsupported`. The summary is built from the new `Rendered.unsupported` field.
2. **Refusal texts:** `render_one`'s unreachable re-check keeps its text, now formatted in the render crate from the signature. The CLI and export refusal texts are left alone.
3. **Out of scope:** the live warning payload and `firstCycle`.
4. **Pinned quirks kept:** the trailing `keys=` in the engine message stays, and the runtime still dedupes live warnings by sound only.
5. **Engine scope:**
   - `capability` and `orbit` become `pub(crate)`.
   - `UnsupportedSignature`, `unsupported_signatures` and `CHAIN_KEYS` are re-exported at the root.
   - `signature_of` goes behind `#[cfg(test)]`.
   - `orbit_pairs.rs` moves into `orbit.rs` unit tests, using `include_str!` on the shared table.
   - `Rendered` gains `unsupported: Vec<UnsupportedSignature>`.
6. **Not needed:** no A/B guard (no DSP change), and no refs, vectors, goldens or host rebuild.
