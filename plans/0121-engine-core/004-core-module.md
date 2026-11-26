# Stage 004: the render core in its own module

## Goal

`Core` and its seam live in `crates/strudeland-engine/src/core.rs`, with private fields.
`Engine` (`render.rs`), `LiveEngine` (`live.rs`) and `StemCapture` (`stems.rs`) use only Core's
methods and `Slot`'s accessors. The compiler enforces that from now on. `render.rs`'s tests read
through the interface, and the Core-only tests move to `core.rs`. The engine README and
AGENTS.md name the module.

## Prerequisites

- Stage 003 is committed, and its A/B check reported 0 differences.
- Read the proposal's "Core's interface" section. Its list of test-only readers is complete.

## Implementation

### 1. Create `core.rs` and move the code into it

- **Add the module** to `lib.rs`: `pub(crate) mod core;`, kept in alphabetical order (between
  `controls` and `delay`).
- **Move these from `render.rs`, unchanged in body:**
  - `MAX_LOG_CHARS`, `missing_duck_target`, `HapTag` (now `pub(crate)`, with `new` and `error`
    `pub(crate)`), `Slot`, `retain_flagged`, `ScheduleArgs`, `Core` and its whole `impl`;
  - `IrTarget`, `IrSettler` and `SyncSettler`.
- **Do not reorder** a statement of `schedule`, `push_log`, `step`, `step_with_stems` or
  `settle_reverbs`. The move is cut and paste.
- **Module doc:** "The render core both drivers share (the offline `Engine` in `render` and
  `live::LiveEngine`): the voices, buses, orbits and caches, scheduled hap by hap and rendered a
  quantum at a time into the mix, and the IR settler seam its reverbs settle through."
- **Leave in `render.rs`:** `LATE_EPSILON`, the frame helpers (`round_up_128`, `frame_for_pause`,
  `frame_for_param_event`, `frame_for_buffer_start`), `zeroed` and `Engine`.
- **No Blink header on `core.rs`.** It ports no Blink file, so its comments must not name a `.cc`
  or `.h` file. `tests/licence.rs` checks this. `render.rs` keeps its header, because it still
  holds the frame conversions.

### 2. Make the fields private

- **`Core`:** every field loses `pub(crate)`. The `#[cfg(test)] seeds` field stays, private.
- **`Slot`:** every field is private. Add:

  ```rust
  /// Its value's index (its bus level and stem group).
  pub(crate) fn value(&self) -> usize { self.value }
  /// The index of its orbit in the bank.
  pub(crate) fn orbit(&self) -> usize { self.orbit }
  #[cfg(test)] pub(crate) fn seq(&self) -> u64 { self.seq }
  #[cfg(test)] pub(crate) fn voice(&self) -> &Voice { &self.voice }
  ```

- **`ScheduleArgs`** keeps its `pub(crate)` fields. It is an argument record with no invariant.
- **Test-only readers on `Core`**, all `#[cfg(test)] pub(crate)`:
  - `errors(&self) -> &[RenderError]`
  - `slots(&self) -> &[Slot]`
  - `seeds(&self) -> &[u32]`
  - `noise(&self) -> &NoiseCache`
  - `ir_cache(&self) -> &IrCache`
  - `orbits(&self) -> &OrbitBank`, which already exists from stage 002
  - `reverb(&self, IrTarget) -> Option<&Reverb>`, which already exists from stage 003

  Add nothing else. A test that needs more belongs in `core.rs`'s own tests.

### 3. The callers

- **`render.rs`:** `use crate::core::{Core, HapTag, ScheduleArgs, SyncSettler};`. `Engine`
  already uses only methods since stages 002 and 003.
- **`live.rs`:**
  - `use crate::core::{Core, IrSettler, IrTarget, ScheduleArgs, SyncSettler};`;
  - its module doc's "on the render core the offline `crate::Engine` uses" stays as it is;
  - the test reads `e.core.slots[0].voice` and `.seq` become `e.core.slots()[0].voice()` and
    `.seq()`.
- **`stems.rs`:**
  - `use crate::core::Slot; use crate::render::{Engine, zeroed};`;
  - in `StemCapture::begin`, use `slot.value()` and `slot.orbit()`.
- **`job.rs`, `osc.rs`, `chain.rs` and `modulate.rs`** keep `crate::render::frame_*`. Those
  helpers did not move.

### 4. `render.rs`'s tests

**Move to `core.rs`'s `#[cfg(test)] pub(crate) mod tests`:**

- `retain_flagged_keeps_the_flagged_items_in_order`;
- `bare_core`, `steps` and `args`;
- `cps_is_per_hap` and `a_growing_graph_moves_a_playing_voice_to_its_new_level`, which keep
  `grown.buses.index_of`, a private read that is legal there;
- every Core test from stages 002 and 003 (`stop_voices_…`, `set_seed_…`,
  `create_orbit_keeps_first_add_order`, `finish_…`, `settle_reverbs_…` and `land_ir_…`) with the
  `settle` helper and the `Recording` settler. Inside `core.rs` these may read private fields
  directly;
- `value_map`, made `pub(crate)`, so that `render.rs`'s tests can import it
  (`use crate::core::tests::value_map;`) for the bus test near `render.rs:1872`.

**Stay in `render.rs`,** with each read rewritten:

| Today | After |
|---|---|
| `e.core.errors` (`.is_empty()`, `.len()`, `[i]`, `.iter()`, `{:?}`) | `e.core.errors()` |
| `e.core.ir_cache.generated()` / `.len()` | `e.core.ir_cache().generated()` / `.len()` |
| `e.core.slots.len()` / `.is_empty()` | `e.core.slots().len()` / `.is_empty()` |
| `e.core.slots[0].voice.seed()` | `e.core.slots()[0].voice().seed()` |
| `e.core.slots.iter().map(\|s\| … s.value …)` / `s.seq` | `e.core.slots().iter()` with `s.value()` / `s.seq()` |
| `e.core.seeds` (compared with a `Vec<u32>`) | `e.core.seeds()`; `driver_seeds` returns `e.core.seeds().to_vec()` |
| `e.core.noise.is_cached(…)` | `e.core.noise().is_cached(…)` |
| `e.core.orbits.orbits()[k]` (`djf`, `reverb`, `output_gain`, `duck_curve`) | `e.core.orbits().orbits()[k]` |
| `e.core.bus_graph.max_level()` / `.level_of_value(…)` | `e.core.bus_graph().…` |
| `Engine::new(&with_bus).unwrap().core.bus_graph.max_level()` | `….core.bus_graph().max_level()` |
| `e.core.settle_reverbs(&mut SyncSettler)` (from stage 003) | unchanged |

Every assertion keeps its text and value. When the stage is done, `rg -n 'core\.[a-z_]+\b[^(]' crates/strudeland-engine/src/render.rs crates/strudeland-engine/src/live.rs crates/strudeland-engine/src/stems.rs`
matches nothing. The compiler enforces the same thing.

### 5. Docs

- **`crates/strudeland-engine/README.md`, "Live mode":**
  - the first sentence's `` (`render::Core`) `` becomes `` (`core::Core`) ``;
  - the "Reverb IRs" bullet gains one sentence: "Both drivers settle through the core's one walk
    (`Core::settle_reverbs`): offline and synchronous live through the synchronous settler
    (`Convolver::new`), the worker through its own settler (`build_ir`, `Convolver::new_live`)."
  - the `stop` bullet stays as it is.
- **`AGENTS.md`, Layout, `crates/strudeland-engine/`:** replace
  `` `render` (the pause loop and voice driver) `` with
  `` `core` (the render core both drivers share: voices, buses, orbits, caches, the IR settler
  seam), `render` (the offline driver: the pause loop and frame conversions) ``. Leave the rest
  of the line as it is.

## Tests

- **Moved:** the Core-only tests and helpers listed in step 4, unchanged in assertions.
- **Rewritten:** the Engine tests' reads, per the table. The live tests' slot reads now go
  through `slots()`, `voice()` and `seq()`.
- **Deleted or added:** none in this stage.

## Validation

- The gates in [000-prompt.md](000-prompt.md), with `N=004`. All of them exit 0. Also run
  `bun test > "$S/0121-004-bun-test.log" 2>&1; echo "exit $?" >> "$S/0121-004-bun-test.log"`
  once, because AGENTS.md changed.
- The A/B log reports 0 differences.
- `cargo test -p strudeland-engine --test licence` passes. It is part of the crates gate, so
  read it in the log.
- `rg -n '^\s+pub\(crate\) [a-z_]+:' crates/strudeland-engine/src/core.rs` lists only
  `ScheduleArgs`'s fields.
- `rg -n '(^|[^:a-z_])core::' crates/strudeland-engine/src/lib.rs` prints nothing: no ambiguous
  bare `core::` path beside the new module.
- `git diff --stat main -- crates/strudeland-live` is empty.
- `git diff --stat main` lists only:
  - `crates/strudeland-engine/` (`src/core.rs`, `src/render.rs`, `src/live.rs`, `src/stems.rs`,
    `src/orbit.rs`, `src/voice.rs`, `src/chain.rs`, `src/reverb.rs`, `src/lib.rs`,
    `tests/live_equals_offline.rs` and `README.md`);
  - `AGENTS.md`;
  - `plans/`.

## Commit message

```
refactor(engine): move the render core into its own module

Core, its slots and the IR settler seam now live in core.rs with
private fields. The offline Engine, LiveEngine and the stem capture
use only its methods and Slot's value and orbit accessors, so the
core's invariants can no longer be bypassed. The Core-only tests move
with it; the drivers' tests read through test-only accessors.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
