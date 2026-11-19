# strudeland-live

Live playback for strudeland: batches of haps in, audio out through a ring to the device, clock
anchors, stats and scope frames back as `Event`s. No Tauri dependency.

`Player::start(output, seed) -> Result<(Player, Receiver<Event>), LiveError>`, then
`schedule(batch)`, `set_seed(seed)` and `stop()` (also on `Drop`).

## Modules

- `player`: `Player` and the engine thread (below).
- `master`: `MasterStage`, the master meter and optional safety limiter (below).
- `ir_worker`: the reverb IR worker thread (`build_ir` off the engine thread).
- `batch`: `Batch`, `ValueEntry`, `HapEntry` (the `playback_schedule` payload) and
  `Batch::validate` (shape checks only: finite numbers, `duration >= 0`, `cps > 0`, no repeated
  value index; a hap naming an unknown index is the engine thread's `Event::Error`).
- `event`: `Event`, `Stats`, `u64_string` (camelCase JSON; `hostNs` is a decimal string).
- `ring`: `RING_CAPACITY_FRAMES` (8192) and `RING_TARGET_FRAMES` (2048), the only places the
  depth is set; `make_ring`, `make_anchor_ring`, and `make_callback`, the audio-thread body.
- `resample`: `QuantumResampler`, 44.1 kHz engine quanta to the device rate (rubato sinc,
  fixed 128-frame input); the identity at 44100 Hz.
- `output`: the `Output` trait and its types. `cpal_output`: `CpalOutput`, the default device,
  stereo `f32`, the stream owned by its own thread (re-queried on every start, so a default-device
  change is followed). `fake`: `testing::FakeOutput` with a virtual clock and `Pace::RealTime` or
  `Pace::AsFastAsPossible` (its `pulled()` keeps the first 2^24 samples).

## Threads

1. **The audio thread** (cpal's, or the fake's) runs the callback from `ring::make_callback`.
   The real-time rule: it pops the ring (zero-filling and counting an underrun when short), bumps
   atomics (`Shared`: primed, fade, underruns, minimum fill, callbacks) and pushes
   `AnchorSample`s into a 16-slot ring (a full ring drops the sample). It never allocates, frees,
   locks, blocks or logs; `tests/rt_alloc.rs` proves it with a counting global allocator. Before
   the ring is primed it writes zeros and counts nothing; at stop it ramps the first 128 frames
   to silence and writes zeros from then on.
2. **The engine thread** (`strudeland-live-engine`) owns the `Output`, the `LiveEngine` (which is
   `!Send`, so it is built there), the ring producer, the resampler and the control receiver. One
   loop pass: drain the controls (batches, seed, stop); poll the output for a device change or a
   failure; forward the engine's warnings and errors; turn anchor samples into `Event::Anchor`;
   render one slice (at most 4 quanta, resampled and pushed, stopping once the ring holds
   `RING_TARGET_FRAMES` device frames); send `Event::Scope` every 33 ms (after the first anchor)
   and `Event::Stats` every second; wait on the control channel, 100 us if the ring is still below
   its target, else 1 ms. Opening a stream fills the whole target before priming it.

   **Scheduling** (`rt`). On macOS the engine thread runs under the Mach time-constraint policy
   (`THREAD_TIME_CONSTRAINT_POLICY`, as CoreAudio's I/O threads do), `preemptible` 1: a period of
   one slice of audio (512 engine frames, 11.6 ms), a computation of half of it and a constraint
   of the whole period. Blocking between slices keeps a catch-up burst
   inside the budget, so the kernel does not demote the thread, and the thread can never take more
   than half a core from the UI. The fake output's pull thread asks for the same policy (a period
   of one block, a quarter of it to compute), standing in for the device's real-time thread. Best
   effort: a refused policy falls back to the user-interactive QoS class and is counted
   (`realtime_refusals()`, which the dropout test prints and on macOS requires to be 0); nothing
   is logged. History: with no priority a machine oversubscribed about twice over starved the
   render-ahead and `dropout_short` failed every run; the QoS class alone passed `dropout_short`
   but not `dropout_full` at load averages over 15 from other projects (tuning log); the
   time-constraint policy passes both.
3. **The IR worker** (`strudeland-live-ir`) builds reverb IRs the engine requests; it ends when
   the engine (and with it the request sender) is dropped. A reverb is silent on its first use
   until the worker answers (the previous IR plays meanwhile on a change).

Batch values are added in arrival order. The engine wants value indices 0, 1, 2, ... with no
gaps, so the engine thread maps each batch index to the next engine index; a value that is not a
control map is one `Event::Error` and its haps are dropped silently; a hap naming an index never
sent is one `Event::Error` per index per batch. `batchDecodeUs` is that work's time, rounded up.

**Device changes.** On `DeviceChanged` the thread emits the old stream's last anchors, stops the
output, and starts a new stream at the device's current rate with a new ring, a new anchor ring
and a resampler for that rate; the old ring's unplayed frames (at most 46 ms) are discarded. A
failure is one `Event::Error`, then a silent reopen once a second until it works. Batches that
arrive while the output is down are dropped (and counted in `batches`).

**Stopping** sets the fade, waits (at most 100 ms) for the ramp and one whole silent buffer
after it, stops the output, drops the engine and joins the worker.

## Anchor rule

`consumed` counts the ring frames the callback popped before this callback in the current stream
(padding does not count). An anchor sample is pushed on the stream's first primed callback, on
the first callback after an underrun and otherwise every 250 ms of consumed frames. The engine
thread sends `Event::Anchor { frame, host_ns }` with

```
frame = max(0, round(base_frame + consumed * 44100 / device_rate - resampler_delay_frames - MASTER_DELAY_FRAMES))
```

`host_ns` is when the callback's first frame is heard (cpal's `timestamp().playback`, mach host
time on macOS), `base_frame` the engine's render head when the stream started (0 for the first),
`resampler_delay_frames` rubato's `output_delay()` in 44.1 kHz frames (0 at 44100 Hz), `MASTER_DELAY_FRAMES` (66) the master stage's lookahead delay. So `frame`
is the engine frame heard at `host_ns`; anchors carry no cps. The offset is signed: a restarted
stream's first anchor (consumed 0) reports a frame before `base_frame`, on the same line as its
later anchors; only a frame before the engine's frame 0 is clamped.

## Master stage

`master::MasterStage::process` runs in place on every 44.1 kHz engine quantum, on the **engine
thread** after the voices are summed and before the resampler and the ring. It never runs in the
output callback, so the real-time rule holds: every buffer is allocated in `MasterStage::new` and
`process` allocates, locks and logs nothing.

- **Measure.** It first measures the quantum as rendered (before any limiting): per channel the
  peak, the rms and the count of samples at or over full scale, plus the lowest limiter gain.
  A non-finite sample counts as an over of peak 1.0, so every `LevelFrame` field is finite. The
  engine thread takes a frame every scope interval (`take_frame`, none when nothing rendered)
  and sends it as `Event::Levels`, anchored or not.
- **Delay.** It then writes the quantum delayed by the lookahead, `MASTER_DELAY_FRAMES` = 66
  frames (1.5 ms at 44.1 kHz). The delay line is always in the path, whether the limiter is on or
  off, so toggling the limiter never moves audio against the clock anchors; the anchor rule
  subtracts the 66 frames. The first 66 frames of a new stream are silence (`reset_stream`).
- **Limit.** When enabled (`Player::set_limiter`, off when a player starts) the output is limited
  to a -1 dBFS sample ceiling: the per-frame required gain, its minimum over the lookahead window,
  an instant-attack exponential release (50 ms), and a box mean over the lookahead, so
  `|y| <= ceiling` holds exactly. The gain computer runs on every frame, on or off; `enabled`
  only decides whether the gain is applied (off, the output is the delayed input bit for bit) and
  reported. Toggling resets nothing, so turning it on limits the frames already in the delay line
  too. The module docs hold the full algorithm; the offline limiter in `strudeland-render` and the JS port in `master-dsp.ts` follow the same spec.

## Ring depth

The ring holds up to 8192 device-rate frames; the engine thread keeps it at 2048 (46 ms at
44.1 kHz, 43 ms at 48 kHz). The hap lead is the cyclist's 0.1 s latency minus the depth minus the
IPC time, so the depth may only rise to 3072, and only if the dropout test shows
underruns with no late haps. The dropout test's pass condition (zero underruns, zero late haps,
zero skipped haps) is never loosened.

### Tuning log

| date | target frames | run | underruns | late haps | skipped | min headroom (last s) | load average (before, 1/5/15 min) | result |
|---|---|---|---|---|---|---|---|---|
| 2026-11-14 | 2048 | `dropout_full`, release, 30 s, 13 busy threads, 48 kHz | 0 | 0 | 0 | 28.4 ms | 6.76 / 16.29 / 21.14 | pass: the depth stays 2048 |
| 2026-11-14 | 2048 | the same, after the QoS change, on a machine loaded by other projects | 3 | 0 | 0 | 39.0 ms | 15.90 / 20.52 / 21.58 (28.64 when the run started) | fail, not acted on: the machine was not quiet (external load alone about 15 on 14 cores). Re-run on a quiet machine before raising the depth |
| 2026-11-14 | 2560 | the same, QoS class only (reverted) | 61 | 10 | 0 | 28.1 ms | 26.25 / 22.46 / 20.62 (38.60 after) | fail: a deeper ring does not help; the engine thread is starved |
| 2026-11-14 | 2048 | the same, engine and fake output threads under the time-constraint policy | 0 | 0 | 0 | 42.7 ms | 8.46 / 19.40 / 20.24 (25.80 after) | pass |
| 2026-11-14 | 2048 | the same | 0 | 0 | 0 | 42.7 ms | 28.30 / 22.33 / 21.23 (42.31 after) | pass |
| 2026-11-14 | 2048 | the same | 0 | 0 | 0 | 42.7 ms | 42.31 / 26.43 / 22.76 (43.15 after) | pass: the depth stays 2048 |
| 2026-11-14 | 2048 | `dropout_short`, debug, 3 s, 2 busy threads, five runs | 0 | 0 | 0 | 42.7 ms | 47.36 falling to 37.19 (1 min) | pass, five of five |

With the time-constraint policy the minimum headroom over the last second stayed at the full
target (2048 frames at 48 kHz is 42.7 ms): the callback never found the ring short of a whole
target. Load averages count the dropout test's own 13 busy threads while it runs.

## Tests

`cargo test -p strudeland-live`. Nothing touches hardware: `tests/player.rs` drives the `Player`
through the fake, `tests/rt_alloc.rs` through an output of its own, and `tests/dropout.rs`
(`dropout_short`, 3 s) plays the synthetic job (`tests/data/live-pressure.job.json`) through the
real-time fake at 48 kHz under load. By hand:

- `cargo test --release -p strudeland-live --test dropout -- --ignored --nocapture`: the full
  dropout test (30 s, every core but one busy); run it on a quiet machine.
- `cargo test --release -p strudeland-live --test ipc_decode -- --ignored --nocapture`: the batch
  decode benchmark.

Licences: cpal (Apache-2.0), rtrb (MIT or Apache-2.0), rubato (MIT or Apache-2.0), serde and serde_json (MIT
or Apache-2.0), thiserror and mach2 (MIT or Apache-2.0). The crate is AGPL-3.0-or-later.

Regenerate `tests/data/live-pressure.job.json` from the scheduler benchmark source using the
planner command at the top of [`tests/bench/live-pressure.js`](../../tests/bench/live-pressure.js).
