# 0134: narrow the engine's public interface

## Goal

`strudeland-engine` exports two modules that nothing outside the crate needs. `capability` has
10 public items; outside the crate only `unsupported_signatures`, `UnsupportedSignature` and (in
two tests) `CHAIN_KEYS` are used. `orbit` has 13 public top-level items; its only outside user
is the integration test `tests/orbit_pairs.rs`. `Rendered` also reports unsupported material as
preformatted `RenderError` text (`"s=… keys=…"`), which the engine binary prints and the render
crate's `render_one` re-reads by kind.

After this change:

- `capability` and `orbit` are `pub(crate)`. The root re-exports `UnsupportedSignature`,
  `unsupported_signatures` and `CHAIN_KEYS` next to `ALLOWED_KEYS`. The root stops exporting
  `Engine` and `quantum_end_time`, which have no outside users.
- `Rendered` gains `unsupported: Vec<UnsupportedSignature>`. A refused job has silent channels,
  empty `errors` and its signatures in `unsupported`. The engine binary builds its
  `{kind:"unsupported",message:"s=… keys=…",bar}` summary entries, exit 1 and
  `ErrorKind::Unsupported` from that field. `render_one` builds its unreachable re-check text
  from it too.

**No visible change and no sound change.** The `signatures` JSON, the `render` summary, stderr
and exit codes, the WAV bytes, the CLI and export refusal texts and the live warning payload all
stay byte-identical. The gates prove it with an A/B capture of the engine binary, a
public-surface listing and literal counts.

## Background

[reverify.md](reverify.md) holds the evidence at `7c6e632`. Its **Decisions** section is
binding. One finding is new since then and changes the file list:

**Making `capability` and `orbit` crate-private makes 35 items dead outside tests.** `Orbit`'s
public methods return `Reverb`, `FeedbackDelay`, `Djf` and `Param`, so their accessors counted
as reachable public API. A probe ran on a copy of the crate in the scratchpad (not a worktree)
with both modules `pub(crate)`. `cargo clippy --lib` then reported 15 dead-code warnings covering
35 items in 7 files. Under `--tests`, 32 of them (`signature_of` among them) are used by unit
tests and 3 are used nowhere: `Orbit.channels` (the field and `Orbit::channels()`) and
`IrPlan::params()`. The crate already has a convention for this case, `#[cfg(test)] pub fn`
(e.g. `orbit.rs:600`, `biquad.rs:284-301`, `compressor.rs:260`). So the plan marks the 32 items
`#[cfg(test)]` and deletes the 3. With
those edits the probe was clean under `--lib`, `--lib --tests` and
`--target x86_64-pc-windows-msvc`. Removing `Engine` and `quantum_end_time` from the root adds no
warning.

## Scope

In (one stage, [001-narrow-engine-interface.md](001-narrow-engine-interface.md)):

1. `crates/strudeland-engine/src/lib.rs`: the module visibilities, re-exports, `Engine` and
   `quantum_end_time`, and the `Rendered` field and its doc.
2. `capability.rs`: `signature_of` behind `#[cfg(test)]`; the `CHAIN_KEYS` doc's link to the
   now-private `CHAIN_UNSUPPORTED_KEYS` becomes plain backticks; the unit test helper
   `sound_render` also asserts that nothing was refused.
3. The test-only accessors in `delay.rs`, `djf.rs`, `orbit.rs`, `param.rs`, `reverb.rs`,
   `voice.rs`; the 3 deletions in `orbit.rs` and `reverb.rs`. `live.rs:2`'s module doc names
   the offline engine as plain `` `Engine` `` instead of the link ``[`crate::Engine`]``, which
   would point at a now-private item (a doc-only edit).
4. `tests/orbit_pairs.rs` moves into `orbit.rs`'s `mod tests` (`include_str!` of the shared
   `tests/helpers/orbit-pairs.json`, as `job.rs:488` already includes `invalid-jobs.json`).
5. `render.rs` (engine): `run_observing_overflow` fills `unsupported` instead of formatting
   errors; its two unit tests assert on the field.
6. The engine binary: the import and the summary built from `unsupported` plus `errors`.
7. Engine tests: `fixtures.rs`, `micro.rs`, `bin.rs`, `never_panics.rs`, `stems.rs` (imports,
   and keep every "rendered cleanly" check as strong as before, see Risks).
8. `strudeland-render`: `plan.rs` and `tests/decay_bound.rs` imports; `render.rs` `render_one`
   and its fake; `pipeline_audio.rs` literals; `tests/joiner_identity.rs` clean-render check.
9. `crates/strudeland-engine/README.md:635-636`: one sentence on where the library reports
   refusals.

Out:

- The `signatures` JSON, the render summary shape, `ErrorKind::Unsupported` (reverify
  Decision 1).
- The render crate's `Unsupported` refusal and its Display text (`error.rs:82-99`), the CLI,
  `SKILL.md` and the export dialog (Decision 2).
- `LiveEvent::Warning`, `firstCycle`, the runtime dedupe and the trailing `keys=` quirk
  (Decisions 3 and 4).
- No refs, vectors, goldens, host bundle or IPC bindings: no `PLANNER_FILES`, no bundled TS and
  no command changes (Decision 6).
- `AGENTS.md` does not change. Its engine bullet (`:231-254`) describes `capability` and
  `orbit` as crate modules, and that stays true. It does not name `tests/orbit_pairs.rs` (it
  ends the test list with "...").

## Technical design

```rust
// lib.rs
pub(crate) mod capability;
pub(crate) mod orbit;
pub use capability::{
    ALLOWED_KEYS, CHAIN_KEYS, UnsupportedSignature, allowed_sounds, controls_json,
    unsupported_signatures,
};
use render::Engine;                       // was `pub use`
pub(crate) fn quantum_end_time(..) -> f64 // was `pub fn`

/// A rendered job: planar output channels of exactly `frames` samples, the render errors and
/// the unsupported signatures. A job with unsupported material is not played: its channels are
/// silent, `errors` is empty and `unsupported` lists what the engine cannot play, as
/// [`unsupported_signatures`] reports it.
pub struct Rendered {
    pub channels: Vec<Vec<f32>>,
    pub errors: Vec<RenderError>,
    pub unsupported: Vec<UnsupportedSignature>,
}
```

- **Engine `run_observing_overflow`** (`render.rs:97-125`): `errors` is `core.finish()` when
  played, else `Vec::new()`. `unsupported` is the detection's list, moved into the struct
  unchanged.
- **Engine binary** (`bin/strudeland-engine.rs:140-155`): after the WAV is written, it builds
  `errors` as each signature mapped to
  `RenderError::new(ErrorKind::Unsupported, format!("s={} keys={}", u.s, u.keys.join(",")), Some(u.first_bar))`,
  followed by `rendered.errors`. `ok`, the `errors` key and the exit code read that list. This
  is today's text, order and 300-character cap, because the same constructor runs on the same
  inputs.
- **`render_one`** (render crate `render.rs:217-223`): on `rendered.unsupported.first()` it
  builds `EngineRenderError::new(ErrorKind::Unsupported, format!("s={} keys={}", ..), None)`
  and fails with `unsupported: {e.message}`. The text is formatted in the render crate, as
  Decision 2 requires, and capped as the engine caps it. The path cannot be reached in
  practice, because `take_jobs` refuses first.
- **`pipeline_audio.rs`**: `render_window`'s joined `Rendered` gets `unsupported: Vec::new()`
  (`render_one` has already failed on any). The destructure at `:111` ends in `..`. The `Ramp`
  fake gets `unsupported: Vec::new()`.

## Stages

One: [001-narrow-engine-interface.md](001-narrow-engine-interface.md). The entry point is
[000-prompt.md](000-prompt.md).

## Risks

- **A text or byte changes.** Three gates guard it. The engine binary's outputs over the 181
  fixture jobs plus 79 hand-made ones are captured before and after: `signatures` stdout,
  stderr and exit, the render summary with `renderMs` zeroed, stderr and exit, the WAV
  SHA-256s, `controls --json`, and two bad-path runs. Base and after must be identical
  (`diff -r`). The literal-count script pins the moved format strings, and `tests/bin.rs:221`
  pins the summary entry.
- **"Rendered cleanly" checks get weaker.** Several tests treated `errors.is_empty()` as "the
  job played". A refused job used to fail that check and would now pass it silently. These are
  `fixtures.rs:252,277`, `never_panics.rs:89` (`exercise` returns a count, which callers compare
  to `Some(0)`), `stems.rs:36-37`, `joiner_identity.rs:89` and `capability.rs`'s unit test
  helper `sound_render` (`:1298-1303`, used by `every_allowed_sound_renders`). Each one also checks
  `unsupported`. `exercise` returns `errors.len() + unsupported.len()`. `micro.rs` and
  `decay_bound.rs` are already guarded by their own `unsupported_signatures` checks.
- **An unintended pub change.** The rustdoc-JSON listing (nightly, hidden items included)
  before and after must differ only in planned ways. Every removed line must be in
  `capability` or `orbit`, or be one of the 5 `Engine`/`quantum_end_time` lines. The added
  lines must be exactly the 13 planned ones. The probe measured 121 removed lines and 364 to 256
  in total (255 without the `Rendered` field the probe lacked).
- **A dead-code set that differs from the probe.** If clippy names any item other than the 35,
  STOP.

## Open questions

1. `s={} keys={}` now appears twice in production code: the engine binary and `render_one`.
   Before, it appeared once, in the engine. A method on `UnsupportedSignature` would leave a
   single copy, but it would add public API and move `render_one`'s formatting into the engine,
   against Decision 2's wording. The plan keeps two copies, both pinned by the literal script.
2. The 32 `#[cfg(test)]` attributes and 3 deletions in `delay`, `djf`, `param`, `reverb` and
   `voice` go beyond the reverify's file list. They follow from `orbit` becoming crate-private.
   The alternative, `#[allow(dead_code)]`, has no precedent in the crate.
