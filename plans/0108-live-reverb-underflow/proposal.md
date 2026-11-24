# Prevent live Rust reverb underruns

## Goal

Restore uninterrupted live Rust playback at a 48 kHz device rate under a reverb-heavy workload.
Identify whether the missed audio deadlines come from impulse-response (IR) construction,
convolver installation, or steady-state convolution. Preserve existing room-change behavior,
tail semantics, and audio timing.

## Scope

- Profile the original reproduction locally with changing rooms, with one fixed room size per
  shared orbit, and with reverb bypassed. Use Superdough as an audible reference.
- Measure again after fixing the costs that drive underruns. Commit an independently generated,
  minimal live regression with no source material from private projects.
- Keep unrelated DSP, transport, and device-buffer tuning outside this change. Include shared
  engine or offline changes only when the measured hot path warrants them.

## Architecture

[Player](../../crates/strudeland-live/src/player.rs) renders into a ring ahead of the output
callback, with resampling for the device rate. The callback records underruns and the lowest
pre-read fill in each stats interval. The engine's render quantum polls completed IRs, settles
new reverb requests, and renders audio. [The IR worker](../../crates/strudeland-live/src/ir_worker.rs)
builds cold IRs asynchronously. [LiveEngine](../../crates/strudeland-engine/src/live.rs) handles
cache hits and worker responses on the render thread. Both paths can clone a fresh convolver.
[Reverb](../../crates/strudeland-engine/src/reverb.rs) applies the newest request and processes
active stage partitions through the tail.

The existing [dropout test](../../crates/strudeland-live/tests/dropout.rs) already exercises
live batch scheduling and resampling to 48 kHz under load. Its fixture uses a fixed room size.
The release-profile short test passed on Apple Silicon with zero underruns, late haps, and
skipped haps. Its final stats interval reported 32.02 ms of minimum ring fill. This establishes
a baseline for the existing fixture.

## Technical Design

1. Capture equivalent release-build sessions on Apple Silicon using a 48 kHz output. Start each
   from a fresh playback session. Hold notes, schedule, seed, routing, and output settings fixed.
   Choose fixed rooms from the sizes used on their respective orbits. If the choice materially
   changes IR length, repeat with the longest size on each orbit to help isolate steady cost.
   Disable all wet sends in the bypass case and verify that the reverb processors are inactive.
   Repeat runs and capture startup through sustained playback.
2. Sample the render and IR worker threads in separate profiling runs from the latency acceptance
   runs. Use bounded local diagnostics if sampling cannot separate short spikes. Correlate
   render-quantum and refill-slice timings with callback ring minima. Track IR request keys, cold
   builds, cache hits, queued and stale results, installations, and time spent cloning or dropping
   convolvers. Measure both worker queue and build time. Measure convolution across several
   partition periods after installation with audible wet input. Compare median, high percentile,
   and worst slice times with nearby ring minima.
   If worker activity and installation overlap, use the engine's existing IR channel test seam
   to deliver a prebuilt result on a known quantum and measure that cost independently.
3. Fix the measured cause. Keep expensive setup and disposal off the render thread if handoff
   dominates. If sustained convolution dominates, measure improvements in the partition work or
   scheduling while preserving output latency and tail behavior. Recheck both paths if they
   contribute independently. Verify that generated IRs remain deterministic and that only the
   newest valid request can replace an active convolver. If common DSP changes, verify existing
   offline and live parity coverage.
4. Reduce an original synthetic workload to the fewest voices, orbits, and room changes that
   expose the failure on the baseline. Reuse the real-time fake device and production scheduling
   path where practical. Verify that wet rendering and any required swaps actually occur within
   the measurement window. A host-sensitive deadline test belongs in the existing opt-in release
   performance suite. Keep deterministic correctness coverage in ordinary tests.
5. Compare the same three cases before and after on the same setup. Report cumulative underruns,
   late and skipped haps, and minimum ring fill across active intervals, starting with the first
   scheduled audio. A run passes at 48 kHz when all three counts are zero and the lowest measured
   fill covers at least one output callback. Resolve intervals without callback samples separately,
   since the existing stats representation reports zero there as well. Confirm continuous
   playback on hardware and require the synthetic regression to show the baseline failure and
   fixed result.

## Dependencies

- Existing 48 kHz output, production Rust playback path, and macOS sampling tools for hardware
  verification. Use the existing fake output for repeatable automated measurements.
- Access to the original reproduction for private validation. Keep the input, modified copies,
  raw captures, and identifying metadata entirely out of tracked files and release artifacts.

## Risks and Open Questions

- Reactive and steady-state work may both miss deadlines. Profiling alone can change scheduling,
  so compare repeated unprofiled runs with the same build and device settings.
- A wet benchmark can appear healthy while IRs are still in flight or the tail has gone silent.
  Verify the active convolver and wet processing before comparing steady cost.
- The room range and callback interval affect the budget. Determine the smallest baseline failure
  through measurement before choosing which automated assertions are stable across hosts.
- Recommend medium complexity. The work spans the live player, engine, and test harness, with
  the final implementation determined by the profile. Skip fracture and retain design
  verification and a light loss check.
