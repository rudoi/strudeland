# 0121 step 0: re-verification on main 3cd77ba

The issue still stands. These are the sites as they are now; `cargo test -p strudeland-engine` and `cargo test -p strudeland-live` both pass.

- **Core has 18 `pub(crate)` fields.** They are at `crates/strudeland-engine/src/render.rs:130-167`: 17 always present plus a `#[cfg(test)] seeds`.
  - The fields: seed, slots, outputs, orbit_of, next_seq, bus_graph, buses, orbits, tables, noise, ir_cache, occurrence, seeds, frame, mix_overflowed, errors, mix, alive.
  - `Slot` (:87-108) is also open: voice, orbit, value, seq, unsettled.
- **`live.rs` reads 11 Core fields outside its tests.**
  - seed: 167, 170, 377
  - noise: 171
  - ir_cache: 172, 326, 332, 379, 419
  - orbits: 190, 343, 351, 427
  - bus_graph: 210, 218
  - frame: 235, 241, 256
  - mix: 265
  - slots: 275, 348, 357, 431
  - outputs: 276
  - orbit_of: 277
  - errors: 284
  - Slot fields: unsettled, voice, seq
- **Callers keep the invariants by hand.**
  - `LiveEngine::stop` clears slots, outputs and orbit_of itself (:274-278).
  - `set_seed` rebuilds `NoiseCache` and `IrCache` itself (:166-173).
  - `LiveEngine::stop` has no production caller: `Player::stop` ends the engine thread instead (`strudeland-live/src/player.rs:361`).
- **Reverb settling is tracked two ways.**
  - Offline uses a slice index: `render.rs:478` takes `first_new = slots.len()`, then :485-491 settle the orbits and `slots[first_new..]`.
  - Live uses the `Slot::unsettled` flag: the synchronous path at `live.rs:342-350`, the worker path at :351-368.
  - The two are equivalent. Both settle before any `step`, so the unsettled slots are always exactly the newest ones, in creation order.
- **The live tests read Core directly.** `live.rs` tests read Core at :542, 685, 719-723, 816, 824 and 878.
- **New since the review:** `stems.rs` reads only `Slot.value` and `Slot.orbit` (`StemCapture::begin`, :256-268). `strudeland-live` uses only the public `LiveEngine` API and `IrRequest`/`IrBuilt`/`build_ir`.

## What must not change (sound)

- **The two paths build different convolvers.** The synchronous settle uses `Reverb::settle_cached`, which builds with `Convolver::new`. The worker path and its fallback use `build_ir`, which builds with `Convolver::new_live`. The live build sums the long tail differently (engine README, "Live mode"). Merging the two changes bits.
- **Settle order:** orbits first, in bank order. Then new voices in creation order, and within each voice its local reverbs in chain order.
- **Settle timing:** offline settles after each pause, live once per quantum, before `step`.
- **Scheduling:** the order of duck, `prepare_value`, `voice_seed`/`occurrence`, the voice build, and summation in `step`.
- **The bit check:**
  - `live_equals_offline` is relative: it passes if both sides move together.
  - The absolute guard is an A/B run. Before the first edit, build the release `strudeland-engine` binary and copy it to the scratchpad. Render every `tests/refs/*.job.json` with it and record each file's sha256 and stderr. After each stage, rebuild and compare.
  - None of `live_equals_offline`'s jobs has a local (FX) reverb. Add `fx-chain-room` and `fixture-fx-chain--mix-*`.
- **Real time:** every Core method runs on the engine thread, never on the output callback (`rt_alloc.rs` checks only the callback). Nothing on the callback path changes.
