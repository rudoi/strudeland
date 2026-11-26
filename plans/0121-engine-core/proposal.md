# 0121: the render core behind its own interface

## Goal

`Core`, the render core shared by the offline `Engine` and the streaming `LiveEngine`, moves to
its own module, `crates/strudeland-engine/src/render_core.rs`, with private fields. Both drivers
use only its interface. The core keeps its own invariants:

- stopping clears the voices, their outputs and their orbit map together;
- reseeding resets the noise and IR caches;
- reverb settling has one new-voice mechanism, the per-slot `unsettled` flag, offline too.

Reverb settling goes through one seam, `IrSettler`. The synchronous settle is one adapter. The
live IR worker is another.

**Nothing audible changes.** Every offline render, every live render (synchronous or through the
worker), every stem and every render error stays the same, bit for bit. Live timing, the real-time
path and `strudeland-live` are untouched.

## Background

The [re-verification](reverify.md) has the evidence, with file and line at
`3cd77ba`. In short:

- **Core has 18 `pub(crate)` fields** (`render.rs:130-167`), and `Slot` (`:87-108`) is open too.
- **`live.rs` reads 11 of those fields outside its tests**, plus `Slot`'s `unsettled`, `voice` and
  `seq`.
- **Callers keep the invariants by hand:**
  - `LiveEngine::stop` clears `slots`, `outputs` and `orbit_of` itself (`live.rs:274-278`);
  - `LiveEngine::set_seed` rebuilds `NoiseCache` and `IrCache` itself (`:166-173`).
- **Reverb settling is tracked two ways.** Offline settles `slots[first_new..]` by index
  (`render.rs:478-491`). Live uses the `unsettled` flag (`live.rs:342-368`). They are
  equivalent: both settle before any `step`, so the unsettled slots are always exactly the
  newest ones, in creation order.
- **The settle code is written three times:**
  - offline (`OrbitBank::settle` and `Voice::settle_local_reverbs`);
  - live synchronous (the same two calls);
  - live worker (`take_reverb_request`, `take_local_reverb_requests`, `request_ir`,
    `build_now`, `apply`).
- **Tests read Core directly:**
  - the live tests in 6 places (`live.rs:542, 685, 719-723, 816, 824, 878`);
  - `render.rs`'s tests in about 60 places.
- **`stems.rs` reads only `Slot.value` and `Slot.orbit`** (`StemCapture::begin`, `:256-268`).

## Scope

In:

1. **A lifecycle on Core** (stage 002). Live and offline use these methods instead of fields:
   - `stop_voices`, `set_seed` and `create_orbit`;
   - the readers `frame`, `mix` and `bus_graph`;
   - the errors: `push_error`, `drain_errors` and `finish`.
2. **The settler seam** (stage 003):
   - `IrTarget` and `IrSettler`, with `Core::settle_reverbs`, `land_ir` and `cache_ir`;
   - the synchronous adapter `SyncSettler`, and the worker adapter in `live.rs`;
   - the `unsettled` flag offline too, so the slice index goes;
   - the deletion of every settle, take and apply helper that becomes dead.
3. **The module** (stage 004):
   - `render_core.rs` with private fields, and `Slot` with private fields and accessors;
   - test-only readers;
   - `render.rs`'s tests rewritten onto the interface, with the Core-only tests moved to
     `render_core.rs`;
   - the engine README and AGENTS.md.
4. **The bit guard** (stage 001): an absolute A/B check against a pre-edit build, plus
   `live_equals_offline` extended to local (FX) reverbs.

Out:

- **Any change to what is built or when.** The two convolver builds stay distinct
  (`Convolver::new` for the synchronous settle, `Convolver::new_live` for the worker and its
  fallback). Settle timing and settle order stay as they are.
- **`strudeland-live`.** Its source, its tests, the IR worker thread and the output callback
  are untouched. It already uses only the public `LiveEngine` API and
  `IrRequest`/`IrBuilt`/`build_ir`.
- **The public API.** `LiveEngine`, `LiveOptions`, `LiveHap`, `LiveEvent`, `LiveCounters`,
  `IrRequest`, `IrBuilt`, `build_ir`, `Engine`, `render` and `render_with_stems` keep their
  signatures and behaviour. `LiveEngine::stop` stays, as a thin wrapper. The one exception:
  `orbit` is a `pub mod`, so the deleted `OrbitBank::settle`, `Orbit::settle_reverb`,
  `Orbit::take_reverb_request` and `Orbit::apply_reverb` are public items today. No crate in the
  workspace calls them (the only outside import of `orbit` is `tests/orbit_pairs.rs`, which
  takes `orbit_pair` and `route`), and the engine is not published, so they go.
- **The scheduling path inside `Core::schedule` and `Core::step`.** That covers the duck,
  `prepare_value`, `voice_seed`/`occurrence`, the voice build and summation. The code moves
  file, but not one statement of it changes order.
- **Refs, vectors, goldens and the host.** No engine output changes, so `engine:refs`,
  `engine:vectors`, `render:check`, `host:build` and `host:parity` are not run. No TS file and
  no `host.inputs.json` file is touched.

## Technical design

### The module and its seams

`render_core.rs` is a **deep module**. Its interface is small: about ten methods, one trait and
one enum.
Behind it sit 18 fields' worth of state and the rules that keep them consistent. The offline
`Engine` (`render.rs`) and `LiveEngine` (`live.rs`) are its two **drivers**. Each one owns a
`Core` and reaches it only through methods.

There are two seams:

- **The driver seam** is Core's methods. A driver decides when to schedule, step, settle, stop
  and reseed. The core decides what those mean.
- **The settler seam** is `IrSettler`. The core walks its reverbs in settle order and hands each
  one to a settler. The settler decides how the IR gets built: synchronously now, or by a worker
  later. There are two adapters:
  - **`SyncSettler`** (in `render_core.rs`), used offline and by live in `sync_ir` mode or with no
    worker;
  - **the worker settler** (in `live.rs`), today's `IrLink` logic.

`Slot` stays a `pub(crate)` type, so `StemCapture::begin(&[Slot])` keeps its signature. Its
fields become private, behind `value()` and `orbit()`.

### Core's interface

```rust
// crates/strudeland-engine/src/render_core.rs
pub(crate) struct Core { /* every field private */ }

impl Core {
    // Lifecycle
    pub(crate) fn new(seed: i32, bus_graph: BusGraph, orbits: OrbitBank) -> Core; // unchanged
    pub(crate) fn add_value(&mut self, map: &ValueMap) -> usize;                  // unchanged
    /// `OrbitBank::create`: a no-op when the number exists, else appended (first-add order).
    pub(crate) fn create_orbit(&mut self, spec: &OrbitSpec);
    pub(crate) fn schedule(&mut self, p: ScheduleArgs<'_>);                        // unchanged
    pub(crate) fn step(&mut self);                                                 // unchanged
    /// Body unchanged. Private today; `pub(crate)` because `Engine::play` calls it from `render.rs`.
    pub(crate) fn step_with_stems(&mut self, capture: Option<&mut StemCapture>);
    /// Every pending reverb request, in settle order, through `settler`: the orbits' reverbs in
    /// bank order, then each unsettled voice in creation order (its flag cleared), its local
    /// reverbs in chain order. Only existing reverbs are visited.
    pub(crate) fn settle_reverbs(&mut self, settler: &mut dyn IrSettler);
    /// Lands a convolver built elsewhere on `target` (newest request wins; a target that is
    /// gone is skipped and `convolver` is not called).
    pub(crate) fn land_ir(&mut self, target: IrTarget, request: u64,
                          convolver: impl FnOnce() -> Convolver);
    /// `IrCache::insert`: caches a convolver built elsewhere.
    pub(crate) fn cache_ir(&mut self, key: IrKey, convolver: Convolver);
    /// Drops every voice: clears slots, outputs and orbit_of together. Never resets the
    /// sequence counter; never touches orbits, the IR cache, occurrence, values or the graph.
    pub(crate) fn stop_voices(&mut self);
    /// Changes the audio seed: resets the noise and IR caches and returns true; the same seed
    /// is a no-op returning false.
    pub(crate) fn set_seed(&mut self, seed: i32) -> bool;

    // Reads
    pub(crate) fn frame(&self) -> u64;
    pub(crate) fn mix(&self) -> &[[f32; QUANTUM]];
    pub(crate) fn bus_graph(&self) -> &BusGraph;

    // Errors
    /// A driver's own error (offline: a late hap), in order with the core's.
    pub(crate) fn push_error(&mut self, e: RenderError);
    /// Live: drains every error (the caller takes at most 32 and drops the rest).
    pub(crate) fn drain_errors(&mut self) -> std::vec::Drain<'_, RenderError>;
    /// Offline: the errors and whether the mix overflowed.
    pub(crate) fn finish(self) -> (Vec<RenderError>, bool);
}
```

Test-only readers, all `#[cfg(test)] pub(crate)`:

- `errors() -> &[RenderError]`, `slots() -> &[Slot]`, `seeds() -> &[u32]`;
- `noise() -> &NoiseCache`, `ir_cache() -> &IrCache`, `orbits() -> &OrbitBank`;
- `reverb(target: IrTarget) -> Option<&Reverb>`, which uses the same lookup as `land_ir`. It is a
  second copy of `land_ir`'s private `target_reverb_mut` lookup, through `&` instead of `&mut`;
  both carry a comment saying so.

On `Slot`:

- `value()` and `orbit()` are `pub(crate)`, for stems;
- `seq()` and `voice()` are `#[cfg(test)]`.

`ScheduleArgs` stays a plain argument record with `pub(crate)` fields. It has no invariant.

`HapTag` moves to `render_core.rs` as `pub(crate)`. `Engine::schedule` builds the late-hap error
with it and passes that error to `push_error`.

`Engine::run_observing_overflow` returns the unsupported errors directly with overflow `false`,
without touching the core. That is what it returns today: nothing has rendered, so
`mix_overflowed` is still `false`. Otherwise it calls `play()` and then `finish()`.

### The settler seam

```rust
// render_core.rs
/// Which reverb an IR request belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IrTarget {
    /// An orbit's reverb: its index in the bank.
    Orbit(usize),
    /// A voice's local reverb: the voice's sequence number and the stage index in its chain.
    Local { seq: u64, stage: usize },
}

pub(crate) trait IrSettler {
    /// Settles one existing reverb at `target`. `seed` is the core's audio seed and `cache` the
    /// run's IR cache.
    fn settle(&mut self, target: IrTarget, reverb: &mut Reverb, seed: i32, cache: &mut IrCache);
}

/// Today's synchronous settle: `reverb.settle_cached(seed, cache)` (a miss builds with
/// `Convolver::new`, the Blink stage layout).
pub(crate) struct SyncSettler;
```

`settle_reverbs` reaches the reverbs through two new accessors, which replace the per-type helpers:

- `Orbit::reverb_mut() -> Option<&mut Reverb>`, `pub(crate)` (`orbit` is public and `reverb`
  is not);
- `Voice::local_reverbs_mut() -> impl Iterator<Item = (usize, &mut Reverb)>`, which yields the
  stage index and the reverb of each `Stage::LocalReverb`, in chain order.

`land_ir` uses `Voice::local_reverb_mut(stage) -> Option<&mut Reverb>`. It returns `None` when
that stage is not a local reverb, which is what `apply_local_reverb` ignores today.

**The worker adapter** (`live.rs`) is today's `request_ir`/`build_now`, rewritten against the
reverb it is handed. For each reverb it is given:

1. **Take the request.** `reverb.take_pending()`, which is today's `take_reverb_request` and
   `take_local_reverb_requests`. Nothing pending means nothing to do.
2. **Key it.** `let key = plan.key(seed)`.
3. **Cache hit:** `reverb.apply(request, || c)`, where `c` is `cache.get(&key)`, a `fresh_live`
   clone, as today.
4. **Link alive:**
   - when the key is not in flight, send an `IrRequest` with the next id and record the key as
     in flight;
   - then record `(target, request, key)` in `waiting`.
5. **Link dead (a send failed):** the adapter marks the link dropped. It builds the miss in place
   (`build_ir`, so `Convolver::new_live`): `reverb.apply(request, || built.convolver.fresh_live())`,
   then `cache.insert(built.key, built.convolver)`. Every later miss in the same settle does the
   same. After `settle_reverbs` returns, `LiveEngine` sets `self.ir = None`, so the next quantum
   settles through `SyncSettler`, as today.

`poll_ir` becomes `IrLink::poll(&mut self, core: &mut Core)`. For each built IR it:

- clears `in_flight` when the ids match;
- lands the IR on every waiting entry with that key, through
  `core.land_ir(target, request, || b.convolver.fresh_live())`;
- then calls `core.cache_ir(b.key, b.convolver)`.

**Equivalence with today's worker path.** Today all requests are taken first and then
processed. The adapter takes and processes each reverb in turn. The results are the same:

- taking one reverb's request reads and writes nothing another reverb's processing uses;
- the cache, the link and `waiting` see the same operations in the same order;
- a cache hit applied to `reverb` lands on the same reverb that `apply(core, target, …)` finds.

### The drivers after the change

- **`Engine::play`:**
  - for each pause: `render_to(pause_frame)`, schedule the pause's haps, then
    `core.settle_reverbs(&mut SyncSettler)`;
  - the `first_new` index goes. The voices from earlier pauses were settled at their own pause,
    so their flags are already clear.
- **`LiveEngine::render_quantum`:**
  - `poll_ir`, then one settle, then `step`;
  - the settle goes through the worker adapter when `!sync_ir` and a link exists, else through
    `SyncSettler`.
- **`LiveEngine::stop`** is `self.core.stop_voices()`. The README documents it, and it has no
  production caller (`Player::stop` ends the engine thread).
- **`LiveEngine::set_seed`** is `self.core.set_seed(seed);`.
- **`LiveEngine::add_value`** creates its orbit with `create_orbit`.
- **`LiveEngine::take_events`** reads `drain_errors().take(MAX_ERRORS_PER_TAKE)`.

Every Core method still runs on the engine thread. Nothing on the output callback changes.

### What is deleted

These become dead once the seam lands (stage 003):

- `OrbitBank::settle`, `Orbit::settle_reverb`, `Orbit::take_reverb_request` and `Orbit::apply_reverb`;
- `Voice::settle_local_reverbs`, `Voice::take_local_reverb_requests` and `Voice::apply_local_reverb`;
- `LocalReverb::settle`;
- `live.rs`'s private `Target`, `apply`, `request_ir`, `build_now` and its `settle_reverbs`;
- the `first_new` index in `Engine::play`.

These stay:

- `Reverb::settle_cached`, `take_pending`, `apply` and the test-only `Reverb::settle`/`Reverb::ir`;
- `IrCache::get` and `IrCache::insert`;
- `build_ir` and `Convolver::new_live`/`fresh_live`.

## What must not change

These come from the reverify's "What must not change (sound)", and each stage restates them:

- **The two convolver builds stay distinct:**
  - the synchronous settle builds with `Convolver::new`, through `IrCache::convolver`;
  - the worker, its fallback and cache hits in worker mode use `Convolver::new_live` and
    `fresh_live`.
- **Settle order:** the orbits' reverbs in bank order, then the new voices in creation order,
  and within each voice its local reverbs in chain order.
- **Settle timing:** offline after each pause's haps are scheduled; live once per quantum,
  after `poll_ir` and before `step`.
- **Scheduling and summation** inside `schedule` and `step`: the code moves unchanged.
- **Error order and text,** including the late-hap error before the hap's own errors, and the
  live limit of 32 errors per take.
- **Real time:** no allocation, lock or log moves onto the output callback (`strudeland-live` is
  untouched, and `rt_alloc.rs` still passes).

## The bit guard

`live_equals_offline` is relative: it passes if both sides move together. The absolute guard is
an A/B run against a build from before any edit. Stage 001 sets it up, and every later stage must
show zero differences.

- **Offline A/B.** Before the first edit, build the release `strudeland-engine` binary and copy it
  to `$S/0121-baseline/bin/strudeland-engine-baseline`, where `$S` is the scratchpad. For every
  `crates/strudeland-engine/tests/refs/*.job.json` (181 jobs), run `render --job … --out …` and
  record:
  - the WAV's sha256;
  - the exit code;
  - stdout with `renderMs` removed (`jq -c 'del(.renderMs)'`);
  - stderr.
- **Live and stems A/B.** The offline binary cannot reach the worker path (`new_live`), `stop`,
  `set_seed` or stems. So a small digest crate in the scratchpad (`$S/0121-baseline/digest/`)
  path-depends on `strudeland-engine`, with the workspace `Cargo.lock` copied in and built
  `--offline`. It prints one FNV-1a digest per variant and job, and `panic` for a variant that
  panics (each runs under `catch_unwind`). The `stop` and `set_seed` actions happen at the quantum
  boundary nearest below half the render, whatever the pauses are:
  - `live-worker`: worker attached, every request built right after the quantum that sent it;
  - `live-sync`: no worker and `sync_ir` false, the synchronous settle `render_quantum` picks
    when no link exists;
  - `live-stop`: `stop()` at mid-render (`sync_ir`);
  - `live-seed`: `set_seed(seed + 1)` and then the same seed again, at mid-render (`sync_ir`);
  - `live-worker-lag`: results three quanta late, each batch in reverse order;
  - `live-worker-stop`: the lag, with `stop()` at mid-render while requests are outstanding;
  - `live-dead`: the worker's request receiver dropped after the first quantum, so every later
    miss builds in place and the link drops;
  - `stems-orbit`, `stems-voice` and `stems-instrument`: `render_with_stems`.

  All of them use only the public API (`LiveEngine`, `IrRequest`/`IrBuilt`/`build_ir`, the
  stems entry points). Its baseline binary is built at stage 001 too, before any engine edit.
- **One script,** `$S/0121-baseline/ab.sh check`, rebuilds both from the working tree, re-runs
  everything and diffs against the baseline. It exits 0 only with zero differences. Nothing under
  `$S` is committed. `ab.sh` and the digest crate's sources are copied to
  `~/.cache/strudeland-0121/`, so the guard survives a cleared scratchpad.

`live_equals_offline` gains the local-reverb jobs `fx-chain-room`, `fixture-fx-chain--mix-0` and
`fixture-fx-chain--mix-1`, after they are confirmed to pass on today's code. None of its current
jobs has a local reverb.

## Tests: replace, don't layer

- **`live_equals_offline`** is kept and extended (stage 001).
- **New Core tests** (in `render.rs`'s test module at stages 002 and 003, moved to
  `render_core.rs` in stage 004):
  - `stop_voices` drops every voice, but an orbit reverb's tail rings on;
  - the sequence number and the voice seeds continue after a stop;
  - orbits, the IR cache and the graph survive a stop;
  - `set_seed` with another seed returns true, resets the noise cache and empties the IR cache,
    and the next settle of a new local reverb generates its IR again (an orbit reverb rescheduled
    with the same settings makes no request, so it cannot show this);
  - `set_seed` with the same seed returns false and keeps both caches;
  - `create_orbit` keeps first-add order, and an existing number is a no-op;
  - `settle_reverbs` visits the orbits and then only the new voices, in creation and chain
    order, and clears their flags. A recording settler shows this;
  - `land_ir` on a gone target does not call the closure.
- **Tests that move:**
  - `set_seed_to_the_same_seed_keeps_the_caches` (live) becomes the Core `set_seed` test;
  - `stop_keeps_an_orbit_reverb_tail` (live) becomes the Core tail test.
- **Live tests that stay** read through the interface:
  - `bus_graph()`;
  - the test readers `orbits()` and `reverb(IrTarget)`;
  - `slots()` with `voice()`/`seq()`.
- **Live worker tests stay** as they are: the newest result wins, a cache hit applies immediately,
  a dropped worker falls back, `sync_ir` ignores a worker, and a local reverb lands through the
  worker. The dropped-worker test gains a local reverb, so one settle exercises the in-place
  fallback for an orbit and a voice.
- **`render.rs`'s Engine tests** read through the test readers. The Core-only tests move to
  `render_core.rs`, where the private fields are visible: `retain_flagged…`, `cps_is_per_hap`,
  `a_growing_graph_moves_a_playing_voice_to_its_new_level` and their helpers.
- **Tests of deleted helpers** (`orbit.rs:1368-1379` and `:1404`, `voice.rs:3494`,
  `chain.rs:3592`, `reverb.rs:2415`) call `reverb_mut()`/`local_reverbs_mut()` and then
  `settle_cached`, with the same assertions.

## Rejected

- **Merging the two convolver builds** into one path. That changes bits: the live build sums the
  long tail in the frequency domain (engine README, "Live mode"). Each adapter keeps its own
  build.
- **A watermark index** (`settled_upto`) instead of the flag. Voices are freed mid-vector by
  `retain_flagged` and dropped by `stop_voices`, so an index would need adjusting on every removal.
  The flag goes with its slot.
- **Changing settle timing,** for example settling live at schedule time, or offline per quantum.
  That moves when an IR lands, which is audible.
- **Deleting `LiveEngine::stop`.** It has no production caller, but it is public API and the
  README documents it. It stays as a one-line wrapper.
- **Staying in `render.rs`.** Rust privacy is per module. While `Engine` and `Core` share a module,
  `Engine` can still reach every field, and nothing enforces the interface.
- **A generic `S: IrSettler` instead of `&mut dyn IrSettler`.** It saves one indirect call per
  reverb per settle, and a settle is not hot. `dyn` keeps `settle_reverbs` a single function.

## Dependencies

- **After #120** (merged as `3cd77ba`).
- **Not needed:** `engine:refs`, `engine:vectors`, `render:vectors`, `render:check`, goldens,
  `host:build`, `host:parity` and `ipc:build`. No TS file, planner file or `host.inputs.json`
  input is touched, and no engine output changes.

## Risks

- **A hidden ordering change in the worker adapter.** Per-reverb take-and-process replaces
  take-all-then-process. The argument above shows they are equal. The `live-worker` digest checks
  it on every refs job, and the worker unit tests check the newest-wins and fallback cases.
- **The offline flag against the index.** These two can disagree only when a voice is scheduled
  outside `play()` and never settled. Some `render.rs` tests do that: they call `e.schedule`
  without `play`, and they never settle afterwards, so nothing changes for them. A later
  `play()` on the same `Engine` would now settle those voices. No test does that. Stage 003 checks
  with a grep.
- **The scratchpad can be cleared** (reboot), and it is per session. Stage 001 copies `ab.sh`
  and the digest crate's sources (not `target/`) to `~/.cache/strudeland-0121/`. If the baseline
  is lost after stage 001 is committed, rebuild it from the stage 001 commit, which changes no
  engine source:
  1. restore `ab.sh` and `digest/` from `~/.cache/strudeland-0121/` into the scratchpad;
  2. make a WIP commit if the tree is dirty;
  3. `git switch --detach <001-sha>`;
  4. `ab.sh rebuild-baseline`, which builds both baseline binaries from the tree and refuses
     unless `git diff 3cd77ba -- 'crates/*/src/*'` is empty; then `ab.sh baseline`;
  5. `git switch -`, and undo the WIP commit.

  Never use a scratch worktree.
- **The licence test.** `tests/licence.rs` requires the Blink header on any file that names a
  `.cc` or `.h` file. `render_core.rs` ports no Blink file: the frame conversions stay in
  `render.rs`. So `render_core.rs`'s comments must not name one. Stage 004 runs that test.
- **The digest crate's dependency versions.** Copying the workspace `Cargo.lock` pins them. In any
  case, the baseline and current digests are built the same way, so a version skew would show up
  as a difference, never hide one.

## Complexity

Medium: about 12 files in one crate, with no new behaviour.

- engine: `render_core.rs` (new), `render.rs`, `live.rs`, `stems.rs`, `orbit.rs`, `voice.rs`,
  `chain.rs`, `reverb.rs` (tests only), `lib.rs` and `tests/live_equals_offline.rs`;
- docs: the engine README and AGENTS.md.

There are four sequential stages, each one commit, each green on its own. The plan is one
document, so there are no fracture or loss-check stages.

## Decisions

1. **`render_core.rs`, with private fields.** The drivers and stems use only its methods and
   `Slot`'s accessors. The module is `render_core`, not `core`, so it never reads as the `core`
   crate and never makes a bare `core::` path in `lib.rs` ambiguous.
2. **One new-voice mechanism:** the private `unsettled` flag on each slot, offline too.
3. **One settle walk,** `Core::settle_reverbs(&mut dyn IrSettler)`, with two adapters.
   `SyncSettler` lives in `render_core.rs`. The worker settler lives in `live.rs`, so the worker
   channels stay live-only.
4. **The settler is handed the `&mut Reverb` itself.** Cache hits and fallbacks apply directly.
   Only results that arrive later need `land_ir` and a target.
5. **Offline errors:**
   - `push_error` for the driver's late-hap error;
   - `finish(self)` for the end of the render;
   - the unsupported path does not touch the core.
6. **Test readers are `#[cfg(test)]`.** Only `frame`, `mix` and `bus_graph` are production
   readers.
7. **The bit guard is absolute:** a pre-edit binary and digest crate, diffed after every stage.
   It adds the live worker (on time, late and reordered, stopped, dead), synchronous, stop, seed
   and stems variants that the offline binary cannot reach.
8. **No GLOSSARY term.** "Render core" and "settler" are engine-internal names. The engine
   README's Live mode section names them.
