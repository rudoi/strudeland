# Stage 003: one settle walk behind the settler seam

## Goal

Reverb settling has one walk, `Core::settle_reverbs`, and two adapters:

- `SyncSettler`, today's synchronous settle;
- the worker settler in `live.rs`, today's `IrLink` path.

The per-slot `unsettled` flag is the one new-voice mechanism, offline too. Built IRs land through
`Core::land_ir` and `Core::cache_ir`. The helpers this leaves dead are deleted.

## Prerequisites

- Stage 002 is committed, and its A/B check reported 0 differences.
- Read the proposal's "The settler seam" and "What is deleted" sections, and `live.rs`'s
  `poll_ir`, `settle_reverbs`, `request_ir`, `build_now` and `apply`, as they are now.

## Implementation

### `render.rs`: the seam (stage 004 moves it to `core.rs`)

1. **`IrTarget`:** move `live.rs`'s private `Target` here and rename it, with the derives, the
   variants and the docs from the proposal. Make it `pub(crate)`.
2. **`IrSettler`:** add the trait with its one method,
   `fn settle(&mut self, target: IrTarget, reverb: &mut Reverb, seed: i32, cache: &mut IrCache)`.
3. **`SyncSettler`:** add the unit struct. Its `settle` is `reverb.settle_cached(seed, cache);`
   and nothing else.
4. **`Core::settle_reverbs(&mut self, settler: &mut dyn IrSettler)`:**

   ```rust
   let (seed, cache) = (self.seed, &mut self.ir_cache);
   for (k, o) in self.orbits.orbits_mut().iter_mut().enumerate() {
       if let Some(r) = o.reverb_mut() {
           settler.settle(IrTarget::Orbit(k), r, seed, cache);
       }
   }
   for slot in self.slots.iter_mut().filter(|s| s.unsettled) {
       slot.unsettled = false;
       for (stage, r) in slot.voice.local_reverbs_mut() {
           settler.settle(IrTarget::Local { seq: slot.seq, stage }, r, seed, cache);
       }
   }
   ```

5. **`land_ir(target, request, convolver)`** calls `apply(request, convolver)` on the reverb from
   a private `fn target_reverb_mut(&mut self, target) -> Option<&mut Reverb>`:
   - for `Orbit(k)`, `orbits_mut().get_mut(k)?.reverb_mut()`;
   - for `Local { seq, stage }`, the slot whose `seq` matches, then
     `voice.local_reverb_mut(stage)`;
   - when the lookup gives `None`, `convolver` is not called.
6. **`cache_ir(key, convolver)`** is `self.ir_cache.insert(key, convolver)`.
7. **`#[cfg(test)] pub(crate) fn reverb(&self, target: IrTarget) -> Option<&Reverb>`** is the
   shared-reference twin of `target_reverb_mut`, with the same lookup.
8. **`Slot::unsettled`'s doc** now reads: "Whether its reverbs' IR requests are still to be
   settled: set at build, cleared by `settle_reverbs`."
9. **`Engine::play`:**
   - drop `first_new` and the two settle statements;
   - after the pause's haps, call `self.core.settle_reverbs(&mut SyncSettler);`;
   - keep the `settleIR()` comment, reworded: "each orbit's newest IR request lands, then the
     local reverbs of the voices not yet settled (this pause's), in creation order".
10. **The stage-002 test helper `settle(core)`:** its body becomes
    `core.settle_reverbs(&mut SyncSettler)`.

### `orbit.rs`, `voice.rs`, `chain.rs`: the accessors and the deletions

Add:

- `Orbit::reverb_mut(&mut self) -> Option<&mut Reverb>`;
- `Voice::local_reverbs_mut(&mut self) -> impl Iterator<Item = (usize, &mut Reverb)> + '_`. It
  yields the stage index and the reverb of each `Stage::LocalReverb`, in chain order;
- `Voice::local_reverb_mut(&mut self, stage: usize) -> Option<&mut Reverb>`. It returns `None`
  when `stage` is out of range or not a local reverb.

Delete:

- `OrbitBank::settle`;
- `Orbit::settle_reverb`, `Orbit::take_reverb_request` and `Orbit::apply_reverb`;
- `Voice::settle_local_reverbs`, `Voice::take_local_reverb_requests` and `Voice::apply_local_reverb`;
- `LocalReverb::settle`. Also check `LocalReverb`'s doc comment for a stale mention.

Drop any import these deletions leave unused (`IrPlan` and `Convolver` in `orbit.rs`). `chain.rs`'s
module doc and `LocalReverb`'s doc say the IR is "settled at the end of the pause through the
run's IrCache". That is still true. Leave it.

Keep:

- `Reverb::settle_cached`, `take_pending` and `apply`;
- the test-only `Reverb::settle` and `Reverb::ir`;
- `IrCache::get` and `IrCache::insert`.

### `live.rs`: the worker adapter

1. **Delete** `Target`, the free function `apply`, `LiveEngine::request_ir`, `build_now` and
   `settle_reverbs`. Import `IrTarget`, `IrSettler` and `SyncSettler` from `crate::render`, and
   `Reverb` from `crate::reverb`.
2. **`IrLink::waiting`** becomes `Vec<(IrTarget, u64, IrKey)>`.
3. **Add the adapter:**

   ```rust
   /// Settles through the IR worker: a cache hit applies now (a fresh_live clone), a miss is
   /// sent (once per key in flight) and waits; once a send fails the link is dropped and every
   /// later miss of this settle is built in place with `build_ir` (`Convolver::new_live`).
   struct WorkerSettler<'a> {
       link: Option<&'a mut IrLink>,
       dropped: bool,
   }

   impl IrSettler for WorkerSettler<'_> {
       fn settle(&mut self, target: IrTarget, reverb: &mut Reverb, seed: i32, cache: &mut IrCache) {
           let Some((request, plan)) = reverb.take_pending() else { return };
           let key = plan.key(seed);
           if let Some(c) = cache.get(&key) {
               reverb.apply(request, || c);
               return;
           }
           let req = match self.link.as_deref_mut() {
               None => IrRequest { id: 0, key, plan, seed },
               Some(link) => {
                   if !link.in_flight.contains_key(&key) {
                       let id = link.next_id;
                       link.next_id += 1;
                       let sent = link.requests.send(IrRequest { id, key: key.clone(), plan, seed });
                       if let Err(e) = sent {
                           self.link = None;
                           self.dropped = true;
                           return build_in_place(reverb, request, e.0, cache);
                       }
                       link.in_flight.insert(key.clone(), id);
                   }
                   link.waiting.push((target, request, key));
                   return;
               }
           };
           build_in_place(reverb, request, req, cache);
       }
   }

   /// A miss built on this thread (the worker is gone): today's `build_now`.
   fn build_in_place(reverb: &mut Reverb, request: u64, req: IrRequest, cache: &mut IrCache) {
       let built = build_ir(req);
       reverb.apply(request, || built.convolver.fresh_live());
       cache.insert(built.key, built.convolver);
   }
   ```

   The statements are today's `request_ir` and `build_now` in the same order. The only change is
   that they apply to the `reverb` they are handed, where the old code looked the target up.
   Write the code so that it reads this way. If the borrow checker forces a restructure, keep
   the same order of cache reads, sends, `in_flight` and `waiting` updates, and builds.
4. **`IrLink::poll(&mut self, core: &mut Core)`** is today's `poll_ir` loop. It calls
   `core.land_ir(*target, *request, || b.convolver.fresh_live())` inside the `waiting.retain`,
   then `core.cache_ir(b.key, b.convolver)`.
5. **`LiveEngine::poll_ir`:** the same `sync_ir` and no-link early returns, then
   `link.poll(&mut self.core)`.
6. **`LiveEngine::render_quantum`:**

   ```rust
   self.poll_ir();
   if !self.options.sync_ir && let Some(link) = self.ir.as_mut() {
       let mut s = WorkerSettler { link: Some(link), dropped: false };
       self.core.settle_reverbs(&mut s);
       let dropped = s.dropped;
       if dropped {
           self.ir = None;
       }
   } else {
       self.core.settle_reverbs(&mut SyncSettler);
   }
   self.core.step();
   ```

   Keep the doc comment: built IRs land first, then the pending requests are settled
   (synchronously) or sent to the worker.

The `build_in_place` path with `link: None` is reached only after a failed send in the same
settle, because `render_quantum` builds `WorkerSettler` only when a link exists. Today's
`request_ir` had the same arm.

After this stage, `live.rs` reads no Core field outside its tests. Check with:
`rg -n 'core\.[a-z_]+\b[^(]' crates/strudeland-engine/src/live.rs`, which should match only
inside `mod tests`.

### Tests

**New**, in `render.rs`'s test module:

1. **`settle_reverbs_visits_orbits_then_new_voices_in_order`**
   - A test settler `Recording(Vec<IrTarget>)` pushes each target, then delegates to
     `SyncSettler`.
   - Setup:
     - a core with orbits 1 and 2 (`OrbitBank::empty(2)` with two `create_orbit`s);
     - value 0 is `{"s":"sine","room":0.5,"roomsize":0.05}` on orbit 1;
     - value 1 is `{"s":"sine","orbit":2,"room":0.5,"roomsize":0.1}`;
     - value 2 is `{"s":"sine","FX":[{"room":0.5,"roomsize":0.05},{"room":0.3,"roomsize":0.2}]}`.
   - Schedule haps of value 0, then 2, then 1. The voices get seq 0, 1 and 2.
   - First settle. Assert:
     - only voice seq 1 (value 2) has local reverbs. Let `a < b` be its two `LocalReverb` stage
       indexes, read from `core.slots[1].voice.stages()`. The targets are
       `[Orbit(0), Orbit(1), Local { seq: 1, stage: a }, Local { seq: 1, stage: b }]`;
     - every slot's `unsettled` is false.
   - Second settle, with no new hap. Assert that the targets are `[Orbit(0), Orbit(1)]`.
   - Schedule one more hap of value 2 (seq 3), then settle. Assert that the targets are the
     orbits followed by seq 3's two locals, and nothing for seq 1.
2. **`land_ir_skips_a_target_that_is_gone`**
   - Call `land_ir(IrTarget::Orbit(7), 1, …)` and `land_ir(IrTarget::Local { seq: 99, stage: 0 }, 1, …)`,
     each with a closure that sets a `Cell<bool>`.
   - Assert that neither flag is set.
   - Then land on a voice's real local-reverb target (built with `Convolver::new` on a one-frame
     IR). Assert:
     - `core.reverb(target).unwrap().applied() == 1`;
     - a second landing with request 1 is ignored: `applied` stays 1 and the closure is not
       called.

**Changed**, in `live.rs`:

- `the_newest_request_wins_whatever_order_results_arrive_in`: replace
  `e.core.orbits.orbits()[0].reverb()` with `e.core.reverb(IrTarget::Orbit(0))`, in both places.
- `a_local_reverb_ir_lands_through_the_worker`, the `applied` closure:
  - find the stage with `e.core.slots[0].voice.stages().iter().position(|s| matches!(s, Stage::LocalReverb(_)))`;
  - then read `e.core.reverb(IrTarget::Local { seq: e.core.slots[0].seq, stage }).unwrap().applied()`.

  Stage 004 turns the field reads into `slots()[0].voice()` and `.seq()`.
- `a_dropped_worker_falls_back_to_a_synchronous_settle`:
  - add value 1, `{"s":"sine","FX":[{"room":0.8,"roomsize":0.3}]}` with no orbit, and schedule
    a hap of it at 0.0 beside the orbit-room hap. Its roomsize differs from `room()`'s 0.5, so
    the key differs and the local request cannot hit the IR the orbit just built. The first
    settle then meets the dead link on the orbit request and builds the local miss in place
    too;
  - additionally assert that the local reverb's `applied()` is 1, through `e.core.reverb(…)`
    after the first quantum;
  - keep `e.ir.is_none()` and the sounding tail.

**Unchanged:** `a_worker_ir_lands_when_polled`, `sync_ir_ignores_an_attached_worker`, and every
offline reverb test except where listed below.

**Moved**, for the deleted helpers. Keep the same assertions in each:

- `orbit.rs:1368`: `assert!(!bank.orbits_mut()[0].settle_reverb(…))`, where orbit 0 has no
  reverb, becomes `assert!(bank.orbits_mut()[0].reverb_mut().is_none())`.
- `orbit.rs:1369` and `:1379`: `bank.settle(0, &mut IrCache::new())` becomes a local helper
  `settle_bank(&mut bank)`. It creates one `IrCache` per call and calls `settle_cached(0, &mut cache)`
  on each `reverb_mut()`, in bank order.
- `orbit.rs:1404`, `voice.rs:3494` and `reverb.rs:2415`: `orbit.settle_reverb(0, cache)` becomes
  `orbit.reverb_mut().unwrap().settle_cached(0, cache)`. Keep the `assert!` on its result.
- `chain.rs:3592`: `v.settle_local_reverbs(0, &mut IrCache::new())` becomes one
  `let mut cache = IrCache::new();` and then `settle_cached(0, &mut cache)` for each
  `(_, r)` in `v.local_reverbs_mut()`.
- `render.rs:1392` (`e.core.orbits.settle(j.seed, &mut e.core.ir_cache)`) becomes
  `e.core.settle_reverbs(&mut SyncSettler)`. That test's voices have no local reverb, so the
  result is the same.

**Before committing,** check the offline-flag risk:
`rg -n 'e\.schedule\(' crates/strudeland-engine/src/render.rs`. No test may call `e.play()` or
`settle_reverbs` on an `Engine` after scheduling haps outside `play`. If one does, report it:
its voices would now be settled where they were not before.

## Validation

- The gates in [000-prompt.md](000-prompt.md), with `N=003`. All of them exit 0.
- **The A/B log reports 0 differences.** The `live-worker` lines are this stage's main check.
- `rg -n 'settle_reverb\b|take_reverb_request|apply_reverb|settle_local_reverbs|take_local_reverb_requests|apply_local_reverb|first_new' crates/strudeland-engine/src`
  prints nothing.
- `git diff --stat main -- crates/strudeland-live` is empty.

## Commit message

```
refactor(engine): settle reverb IRs through one settler seam

Core::settle_reverbs walks the orbits' reverbs and then the voices not
yet settled, and hands each reverb to an IrSettler. The synchronous
settle used offline and in sync mode is one adapter; the live IR worker
is the other, landing its results through land_ir and cache_ir. Offline
now uses the same per-voice flag as live instead of a slice index, and
the per-type settle, take and apply helpers are gone. Each path still
builds its convolver exactly as before.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
