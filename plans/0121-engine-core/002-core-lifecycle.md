# Stage 002: Core's lifecycle, readers and errors

## Goal

Core keeps its own stop and reseed invariants. Both drivers use methods for everything except
reverb settling, which is stage 003. The fields stay `pub(crate)` until stage 004. This stage
adds the methods and moves the callers onto them.

## Prerequisites

- Stage 001 is committed. `$B/base/` exists, and `ab.sh check` reported 0 differences.
- Read the proposal's "Core's interface" section.

## Implementation

### `crates/strudeland-engine/src/render.rs`: new `Core` methods

Add these to `impl Core`, each with a doc comment as in the proposal:

```rust
pub(crate) fn create_orbit(&mut self, spec: &OrbitSpec) { self.orbits.create(spec) }

pub(crate) fn stop_voices(&mut self) {
    self.slots.clear();
    self.outputs.clear();
    self.orbit_of.clear();
}

pub(crate) fn set_seed(&mut self, seed: i32) -> bool {
    if seed == self.seed {
        return false;
    }
    self.seed = seed;
    self.noise = NoiseCache::new(seed);
    self.ir_cache = IrCache::new();
    true
}

pub(crate) fn frame(&self) -> u64 { self.frame }
pub(crate) fn mix(&self) -> &[[f32; QUANTUM]] { &self.mix }
pub(crate) fn bus_graph(&self) -> &BusGraph { &self.bus_graph }

pub(crate) fn push_error(&mut self, e: RenderError) { self.errors.push(e) }
pub(crate) fn drain_errors(&mut self) -> std::vec::Drain<'_, RenderError> { self.errors.drain(..) }
pub(crate) fn finish(self) -> (Vec<RenderError>, bool) { (self.errors, self.mix_overflowed) }
```

Rules for these methods:

- **`stop_voices`** leaves alone `next_seq`, `orbits`, `ir_cache`, `occurrence`, `noise`,
  `tables`, `bus_graph`, `buses`, `frame`, `mix` and `errors`. Its doc comment says so. `alive`
  is rebuilt by every `step`, so it needs no clearing.
- **`set_seed`** is today's `LiveEngine::set_seed` body, moved.

Add this test-only reader:

- `#[cfg(test)] pub(crate) fn orbits(&self) -> &OrbitBank`.

### `render.rs`: `Engine`

- **`run_observing_overflow`:**
  - compute `unsupported_signatures_in(job, self.core.bus_graph())`;
  - when it is not empty, the result's errors are the mapped `RenderError`s and the overflow flag
    is `false`. That is the value today's `self.core.mix_overflowed` has, since nothing has
    rendered;
  - otherwise call `self.play()` and take `(errors, overflowed)` from `self.core.finish()`;
  - destructure `self` so that `core`, `capture` and `channels` move separately. The capture's
    channels are still appended to `channels` before the return.
- **`schedule`:** the late-hap error goes through `self.core.push_error(tag.error(…))`. It is the
  same error, pushed at the same point.
- **`render_to`:** use `self.core.frame()` and `self.core.mix()`.
- **`play`** keeps its settle code unchanged until stage 003.

### `crates/strudeland-engine/src/live.rs`

The callers that move:

- `set_seed`: becomes `self.core.set_seed(seed);`. Keep the doc comment. Drop the `NoiseCache`
  import and, if it is now unused, the `IrCache` import.
- `add_value`: `self.core.create_orbit(&o)`.
- `schedule`: `self.core.bus_graph().cycle_generation()` and
  `self.core.bus_graph().cyclic_readers()[v]`. `self.core.frame()` for `now` and `now_frame`.
- `now_frame`: `self.core.frame()`.
- `render_quantum`: `let mix = self.core.mix(); let (l, r) = (&mix[0], &mix[1]);`.
- `stop`: `self.core.stop_voices();`. Keep the doc comment.
- `take_events`: `self.core.drain_errors().take(MAX_ERRORS_PER_TAKE).map(…)`. The `Drain` is
  dropped at the end of the statement, so the untaken errors are dropped, as today.

After this stage, `live.rs` touches only these fields directly outside its tests: `seed`,
`ir_cache`, `orbits` and `slots`, all in `poll_ir`, `settle_reverbs`, `request_ir`, `build_now`
and `apply`. Stage 003 removes them.

### Tests

These go in `render.rs`'s test module, next to `bare_core`. Stage 004 moves them to `core.rs`.

Add a test helper that settles a core the way offline does today:

```rust
/// Settles the core's pending reverb requests as the offline driver does at a pause.
fn settle(core: &mut Core) {
    core.orbits.settle(core.seed, &mut core.ir_cache);
    for slot in &mut core.slots {
        if std::mem::take(&mut slot.unsettled) {
            slot.voice.settle_local_reverbs(core.seed, &mut core.ir_cache);
        }
    }
}
```

Stage 003 replaces its body with `core.settle_reverbs(&mut SyncSettler)`. Then add these tests:

1. **`stop_voices_drops_every_voice_and_keeps_the_rest`**
   - Setup: `bare_core(0)` and one value `{"s":"sine","room":0.5,"roomsize":0.05}`. Schedule two
     haps, `settle`, then `steps(&mut core, 2)`.
   - Record `core.orbits().count()`, `core.orbits().orbits().len()`, `core.ir_cache.len()`,
     `core.bus_graph().max_level()` and `core.frame()`.
   - Call `stop_voices`, then assert:
     - `slots`, `outputs` and `orbit_of` are empty;
     - every recorded value is unchanged.
   - Schedule a third hap, then assert:
     - its slot's `seq` is 2;
     - `core.seeds` has 3 entries and equals the seeds of a core that never stopped. That core
       schedules the same three haps with no stop in between; build it as a second `bare_core`.
2. **`stop_voices_keeps_the_orbit_reverb_tail`**
   - This replaces live `stop_keeps_an_orbit_reverb_tail`. Delete that test.
   - Setup: `bare_core(0)` and the value `{"s":"sine","room":0.8}`. Schedule one hap
     (duration 0.05), `settle`, then `steps(&mut core, 4)`.
   - Call `stop_voices`, then assert that `steps(&mut core, 20)` has a non-zero sample: the tail
     rings.
3. **`set_seed_resets_the_caches_and_the_same_seed_is_a_no_op`**
   - This replaces live `set_seed_to_the_same_seed_keeps_the_caches`. Delete that test.
   - Setup: `bare_core(0)` and two values, `{"s":"sine","room":0.8,"roomsize":0.5}` and
     `{"s":"white"}`. Schedule one hap of each, `settle`, then `steps(&mut core, 1)`.
   - Assert:
     - `core.ir_cache.len() == 1`;
     - `core.noise.is_cached(NoiseKind::White)`.
   - Call `set_seed(0)`. Assert:
     - it returns `false`;
     - the IR cache still has 1 entry, and white noise is still cached.
   - Call `set_seed(5)`. Assert:
     - it returns `true`;
     - `core.ir_cache.len() == 0` and white noise is no longer cached.
   - Schedule the room hap again, then `settle`. Assert:
     - `core.ir_cache.generated() == 1`: the cleared cache generated the IR again;
     - `core.ir_cache.len() == 1`.
4. **`create_orbit_keeps_first_add_order`**
   - Setup: `Core::new(0, BusGraph::default(), OrbitBank::empty(2))`.
   - Call `create_orbit` with orbits 3, 1, 3 and 2.
   - Assert that `core.orbits().orbits()`'s numbers are `[3.0, 1.0, 2.0]`.
5. **`finish_returns_the_errors_and_the_overflow_flag`**
   - Setup: `bare_core(0)`.
   - Call `push_error` with two errors.
   - Assert that `finish()` returns them in order, with `false`.

The live tests that change, in `live.rs`:

- **`orbits_are_created_in_first_add_order`:** read
  `e.core.orbits().orbits().iter().map(|o| o.orbit())`. Keep its assertions, including the
  unsupported-value case.
- **`a_later_value_that_raises_a_level_renders_finite`:** read
  `e.core.bus_graph().level_of_value(0)`.
- **`set_seed_to_the_same_seed_keeps_the_caches`:** deleted. It moves to Core test 3.
- **`stop_keeps_an_orbit_reverb_tail`:** deleted. It moves to Core test 2.
- **`set_seed_changes_a_noise_voice`** and **`stop_drops_the_voices_and_keeps_the_values`:**
  unchanged. They test `LiveEngine`'s own wrappers.

These stay as they are until stage 003: the reads at `live.rs:816`, `:824` and `:878` (the
reverb and slot reads), and `e.ir`, which is `LiveEngine`'s own field.

## Validation

- The gates in [000-prompt.md](000-prompt.md), with `N=002`. All of them exit 0.
- The A/B log reports 0 differences.
- `rg -n 'core\.(noise|bus_graph|frame|mix|outputs|orbit_of|errors)\b[^(]' crates/strudeland-engine/src/live.rs`
  prints nothing outside `mod tests`. Method calls such as `bus_graph()` don't match. The settle and IR code (`poll_ir`, `settle_reverbs`,
  `request_ir`, `build_now` and `apply`) still reads `core.seed`, `core.ir_cache`,
  `core.orbits` and `core.slots`. Stage 003 removes those reads.
- `git diff --stat main -- crates/strudeland-live` is empty.

## Commit message

```
refactor(engine): give the render core its lifecycle methods

Stopping, reseeding and orbit creation now live on Core, which keeps
their invariants itself: stop_voices clears the voices, their outputs
and their orbit map together, and set_seed resets the noise and IR
caches. Both drivers read the frame, the mix and the bus graph through
methods, and take errors through drain_errors and finish.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
