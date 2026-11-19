# Twin gate recalibration after deterministic voices

This historical calibration preserves the aggregate measurements used to set the thresholds.
The archived reports retain aggregate decisions and reviewed synthetic fixture examples.

Probes run 2026-11-10/11 on Playwright 1.64.1 and Chromium 156.0.8078.4, the same as 0008. They use
`twin-null.probe.ts`, unchanged: chunk 0 of every fixture, three renders, concurrency 4.
For new runs, `PROBE_EVIDENCE_DIR` selects a scratch directory and `PROBE_LABEL` names the run.

```
PROBE_EVIDENCE_DIR=target/twin-probe PROBE_LABEL=rerun \
  bun scripts/render/probes/twin-null.probe.ts
```

## Runs

| Run (file) | Render path | Wall time |
|---|---|---|
| 0008 `twin-null.json` (baseline) | pooled nodes, worklet seed by message | 7.2 min |
| `twin-null-pool-off.json` | node pooling off in the offline render | 7.6 min |
| `twin-null-seeded.json` | + supersaw and wavetable seeded at construction | about 7 min |
| `twin-null-final.json` | + reverb keeps the latest IR | 7.0 min |

Turning pooling off did not slow the render down measurably.

Worst residual by synthetic fixture, in dB, over the three pairs. Reverb-long, reverb-change,
duck, odd-meter, bass-line, stems-shared, silence-entry-perc, silence-entry-pad, noise-seed,
song-linear and song-clip were bit-equal in every run.

| Fixture | 0008 | pool-off | seeded | final |
|---|---|---|---|---|
| worklets | -26.71 | -143.79 | -144.63 | -144.96 |
| pad-seam, stems-5, samples, clipping | -140 to -161 | same | same | same |

## Causes found

1. **Node pool.** Voices hand filters, compressors and
   worklets back to superdough's pool from `ended` callbacks. Those callbacks reach the main thread
   at load-dependent times during an offline render, so whether a voice got a fresh node or a reused
   one, with its old filter state, varied from run to run. The fix is the `node-pool-toggle` vendor
   patch, with `renderOffline` turning pooling off. This also fixed worklets, even though the
   supersaw seed still went by message: fresh nodes happened to receive the message in time.
2. **Worklet seed by message.** The supersaw and wavetable seeds, and the wavetable frames, now
   arrive in `processorOptions` (the `audio-rng` patch, `getWorkletFromPool`). No fixture changed
   measurably beyond run 1. It removes the race, it did not cause a measured difference.
3. **Reverb IR order.** Differing room sizes on one orbit start asynchronous IR renders. The
   convolver originally kept whichever one finished last. Stem isolation showed that changing
   the completion order could affect the render at voice level. The `reverb-latest-ir` patch
   keeps the most recently requested IR that has finished. That also lets live playback use an
   available IR while more recent requests are pending.

Not found: the "white noise seed depends on call order" lead in the task. Noise buffers are cached
per seed, type and sample rate, and `voiceSeed` runs before `superdough()`'s first `await`.

## What is left

- **Float-level differences (-125 to -162 dB) on 12 fixtures.** They remain, so
  `TWIN_BITEXACT_HOLDS` stays false and the twin gate stays in tolerance mode. The stems runs show
  them on most orbits from the first second, so they are not tied to a voice. Their source is not
  identified.
- **Load-sensitive mix-bus residual.** The worst residual in the full run was about -80 dB,
  compared with about -140 dB in two isolated reruns. The largest band difference was 0.003 dB.
  This variation informed the decision to use -60 dB for `TWIN_RESIDUAL_DB`.

## Constants

Rules as in 0008 (`twinDecision`, `scripts/render/probes/calibrate.ts`), from `twin-null-final.json`.

| Constant | Before | After | Evidence |
|---|---|---|---|
| `TWIN_BITEXACT_HOLDS` | false | false | 12 fixtures differ at float level |
| `TWIN_P95_DB` | 0.35 | 0.05 | ceilTo(1.25 x 9e-7, 0.05): one step |
| `TWIN_MAX_DB` | 6.2 | 0.1 | ceilTo(1.25 x 0.003, 0.1): one step |
| `TWIN_RESIDUAL_DB` | -18 | -60 | set by hand: the rule gives -77 (ceil(-80.28 + 3)), but that worst value is load-dependent (alone -139.8 dB), so 3 dB of headroom would make `render:check` flaky. -60 still fails every voice-level difference 0008 measured (-21 to -48 dB) |
| `PROFILES.self.bandMeanDb` | 0.53 | 0.1 | floor (2 x p95 is about 0) |
| `PROFILES.self.bandP95Db` | 0.79 | 0.5 | floor |
| `PROFILES.self.lufsDb` | 0.08 | 0.05 | floor (worst lufsDelta 0) |
| `PROFILES.self.peakDb` | 0.1 | 0.1 | floor (worst peak delta 0) |

The seam and mutation probes were not re-run: their decisions do not read the twin constants, and
the seam twin floor only escalates a fail. The twin gate unit tests were updated for the new limits.
Noise at -40 dB now fails on the residual. A tolerance pass is shown with -90 dB noise instead.

## Goldens

The first full `bun run render:check` had 22 passes and one parity failure because a golden
captured an outcome of the reverb IR race. Regenerating that golden resolved the failure.

A second full `bun run render:check` (all fixtures, mutations on, concurrency 4) was green: 23 pass,
3 mutations caught.
