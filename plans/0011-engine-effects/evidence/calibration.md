# Effect engine calibration

This historical calibration preserves the aggregate measurements used to set the thresholds.
The archived reports retain aggregate decisions and reviewed synthetic fixture examples.

Probes ran on 2026-11-12 (UTC) with Playwright 1.64.1, Chromium 156.0.8078.4 and
`strudeland-engine 0.1.0` on an Apple M3 Max (macOS arm64). See section 6 for load variation.
Re-run after a Playwright bump, a Strudel sync or a change in engine residuals.

These are historical probe commands and report names. Set `PROBE_EVIDENCE_DIR`
to a scratch output directory under the configured Cargo target for a new run.

```
bun scripts/render/probes/engine-null.probe.ts                       # engine-null.json (calibration, 08:49 UTC)
PROBE_LABEL=final bun scripts/render/probes/engine-null.probe.ts     # engine-null-final.json (check, 09:01 UTC)
bun scripts/render/probes/worklet-determinism.probe.ts               # worklet-determinism.json (final refresh)
cargo test -p strudeland-engine --test fixtures -- --nocapture       # engine-fixtures-envelope.json
```

`engine-null.probe.ts` renders every `rust: "supported"` corpus fixture (34) and every micro case
(134) with both engines at concurrency 4 (`DEFAULT_CONCURRENCY`, about 12 minutes). The residual is `10*log10(sum((superdough - rust)^2) /
sum(superdough^2))` over every sample of every channel, no alignment search. The per-group runs of
stages 4 to 12 (`engine-null-micro-<group>.json`) set the micro thresholds; this calibration
re-checks all of them and closes the profile and the render times. Nothing in `parity.ts`,
`thresholds.json` or `thresholds.ts` changed: the two runs' `decision.engine` are identical and
equal to `PROFILES.engine`.

## 1. Micro cases

All 134 cases meet their target (-60 dB; delay, reverb, `measure-feedback-echo`, `fx-chain-delay`,
`fx-chain-room` -40 dB) and their committed `maxResidualDb` (`decision.microTargetsMet` and
`microThresholdsMet` true in both runs; no case is platform-skipped on macOS, so no `null`
verdict). The table summarizes original residuals for cases unaffected by the pulse change
(`-inf` means bit-identical). See the [pulse calibration](pulse-replacement-calibration.md) for
the current pulse and its dependent cases.

| Family | Cases | Residual dB | Target dB |
|---|---|---|---|
| adsr | 6 | -inf | -60 |
| bus | 2 | -inf | -60 |
| coarse | 2 | -inf | -60 |
| compressor | 4 | -inf | -60 |
| crush | 2 | -inf | -60 |
| delay | 6 | -inf | -40 |
| distort | 5 | -inf | -60 |
| djf | 4 | -inf | -60 |
| duck | 6 | -inf | -60 |
| env | 1 | -inf | -60 |
| filter | 13 | -inf | -60 |
| fm | 3 | -inf | -60 |
| fx | 4 | -inf (3), -140.08 (1) | -60, -40 |
| fxrelease | 1 | -inf | -60 |
| ladder | 4 | -inf | -60 |
| level | 2 | -inf | -60 |
| lfo | 1 | -inf | -60 |
| measure | 4 | -inf | -60, -40 |
| mod | 10 | -inf (9), -130.52 (1) | -60 |
| noise | 6 | -inf | -60 |
| orbit | 1 | -inf | -60 |
| osc | 11 | -inf | -60 |
| pan | 3 | -inf | -60 |
| param | 3 | -inf | -60 |
| phaser | 2 | -inf | -60 |
| pitch | 2 | -inf | -60 |
| reverb | 5 | -152.88 to -140.51 | -40 |
| shape | 2 | -inf | -60 |
| supersaw | 6 | -inf | -60 |
| transient | 2 | -inf | -60 |
| tremolo | 2 | -inf | -60 |
| vowel | 2 | -inf (1), -145.52 (1) | -60 |

Not bit-identical, as recorded at their stages: the reverb cases and `fx-chain-room` (#10's
Accelerate reductions in the IR normalization and convolver), and the two measured-floor cases.

**Floors** (07 measured-floor rule, `index.json` `floorDb`, pass within `floorDb + 6 dB`):

| Case | `floorDb` | Threshold | This run | Reason |
|---|---|---|---|
| `vowel-formants` | -145.12 | -139 | -145.52 (`meetsFloorRule` true) | the five formant bandpass outputs sum into `Gain(8)` in hash-set order (5 sounding inputs) |
| `mod-lfo-note` | -129.45 | -124 | -130.52 (`meetsFloorRule` true) | oscillator, LFO and FM inputs sum into `frequency` in hash-set order (3 sounding inputs) |

Eight repeats per worklet case did not show stale `currentTime`. The measured-floor case
`mod-lfo-note` varied with summing order. The current oscillator has separate repeatability
results in the [pulse calibration](pulse-replacement-calibration.md).

## 2. Fixtures

Every supported fixture passes every gate on the Rust render with the unchanged `thresholds.ts`
constants (`decision.gatesHold`) and parity against its superdough golden. Columns: null residual
of the Rust render against the superdough render of the same request; the worst 100 ms window (at
s); the superdough twin floor (two superdough renders of chunk 0); the parity deltas against the
golden (peak / band mean / band p95, dB); the Rust render time (sum of chunk `renderMs`, s, at
concurrency 4). Measurements for unchanged fixtures:

| Fixture | Residual dB | Worst window dB (at s) | Superdough twin floor dB | Parity vs golden | Rust s |
|---|---|---|---|---|---|
| pad-seam | -123.15 | -121.25 (1.1) | -147.93 | 0 / 0 / 0 | 3.44 |
| reverb-long | -95.38 | -93.47 (35.6) | -inf | 0 / 0 / 0 | 18.86 |
| reverb-change | -97.96 | -94.99 (30.2) | -inf | 0 / 0 / 0 | 8.56 |
| worklets | -143.99 | -143.3 (11) | -145.24 | 0 / 0 / 0 | 0.15 |
| duck | -inf | -inf (0) | -inf | 0 / 0 / 0 | 0.01 |
| odd-meter | -776.33 | -734.54 (4.4) | -inf | 0 / 0 / 0 | 0.02 |
| bass-line | -inf | -inf (0) | -inf | 0 / 0 / 0 | 0.01 |
| stems-5 | -147.75 | -144.81 (8.5) | -147.34 | 0 / 0 / 0 | 0.05 |
| stems-shared | -inf | -inf (0) | -inf | 0 / 0 / 0 | 0.01 |
| silence-entry-perc | -inf | -inf (4) | -inf | 0 / 0 / 0 | 0.0 |
| silence-entry-pad | -inf | -inf (2) | -inf | 0 / 0 / 0 | 0.0 |
| clipping | -162.37 | -153.73 (5.5) | -161.56 | 0 / 0 / 0 | 0.01 |
| sine-basic | -156.94 | -146.37 (5.5) | -155.33 | 0 / 0 / 0 | 0.01 |
| song-linear | -inf | -inf (0) | -inf | 0 / 0 / 0 | 0.02 |
| song-clip | -inf | -inf (0) | -inf | 0 / 0 / 0 | 0.03 |
| voice-osc | -146.13 | -143.07 (9) | -146.22 | 0 / 0 / 0 | 0.03 |
| voice-mod | -151.96 | -144.6 (14) | -151.06 | 0 / 0 / 0 | 0.03 |
| voice-noise | -149.39 | -144.43 (13.3) | -150 | 0 / 0 / 0 | 0.01 |
| voice-filter | -136.7 | -120.37 (13) | -136.72 | 0 / 0 / 0 | 0.03 |
| voice-level | -148.69 | -144.24 (14) | -149.01 | 0 / 0 / 0 | 0.04 |
| fx-delay | -756.63 | -732.16 (9.4) | -163.03 | 0 / 0 / 0 | 0.01 |
| fx-room | -123.2 | -109.83 (13.3) | -153.29 | 0 / 0 / 0 | 0.59 |
| fx-native | -146.32 | -143.42 (2) | -147.48 | 0 / 0 / 0 | 0.07 |
| fx-mod | -149.02 | -143.7 (9.6) | -151.87 | 0 / 0 / 0 | 0.08 |
| fx-bus | -149.53 | -144.96 (5) | -152.66 | 0 / 0 / 0 | 0.08 |
| fx-chain | -121.3 | -103.65 (10.5) | -128.52 | 0 / 0 / 0 | 0.26 |

The native effects, modulators and buses sit at their superdough twin floors. The FX chain
(−121.30 dB residual, −128.52 dB twin floor) carries a local reverb, so the engine's Accelerate
rounding difference applies.

Between the two runs, residuals and twin floors that depend on Chromium's summing order or on
superdough's main-thread cleanup timing moved (voice-filter -148.71 then -136.70 dB; fx-delay
-166.34 then -756.63 dB); every parity delta, gate verdict and micro verdict was identical, and
so was the decision.

## 3. `PROFILES.engine`

`decision.engine` in both files: `frameTolerance` 0, `maxOffsetSec` 0.01, `peakDb` 0.1, `lufsDb`
0.05, `bandMeanDb` 0.1, `bandP95Db` 0.5, `hashMustMatch` false. Worst measured: frame delta 0,
offset 0, peak 0.01 dB, LUFS 0, band mean 0.02 dB, band p95 0.13 dB. These are the same worst values
measured in #10. Every value is within today's
constants, so `src/lib/render/parity.ts` is unchanged and its comments keep citing
`plans/0010-rust-engine-core/evidence/engine-null-final.json`.

## 4. Fixture envelope guard and render times

`engine-fixtures-envelope.json`: over the 36 supported synthetic fixture chunk jobs (the five new
fixtures add seven) the worst window mean is 0.003 dB, p95 0.005 dB, peak 0.000 dB, the same as
#10 (the rounding of the stored envelope). The `fixtures` tolerances of `thresholds.json` (0.05 /
0.1 / 0.05 dB, floor -60 dBFS) are unchanged and keep citing #10's file.

Render times (`decision.renderTimesSec`, Rust chunk `renderMs` summed, at concurrency 4, final run):
fx-chain 0.26 s (0.32 s) for 24 s. Standalone, one process at a time (`/usr/bin/time -l`, release build):

| Job | Audio s | `renderMs` | Peak memory footprint |
|---|---|---|---|
| fx-chain chunk 0 (3 runs) | 12.0 | 142 to 159 | 34.4 to 34.8 MB |
| fx-chain chunk 1 (3 runs) | 12.0 | 113 to 120 | 31.3 to 33.9 MB |

The fx-chain fixture's per-voice reverbs clone the cached
convolver per voice (05 measure item 4) and stay at about 35 MB and a fiftieth of real time, so the
clone is not a cost worth optimising. Neither `cargo test --workspace` nor `render:check --engine
rust` approaches three times its pre-task time, so no optimisation is in scope (07). (Process
spawns on this machine sometimes stall for 40 s to several minutes in the loader before `main`;
the wall-clock times of those runs are not render times and are not used.)

## 5. Measure decisions

Every (measure) item of the task, closed at its stage and summarised here. "Rule kept" means the
capture agreed with the documented rule and the engine was ported to it unchanged.

| Item | Capture | Evidence | Decision |
|---|---|---|---|
| bmod twin | `bmod-twin.probe.ts unpatched` then `patched` | `bmod-unpatched-twin.json` (6 renders: all 15 pairs differ, worst -17.79 dB), `bmod-patched-twin.json` (0 pairs differ) | rule kept: the `bmod-gate` patch makes the graph bit-identical; the engine ports the patched graph |
| worklet start | `measure-worklet-start` | `measure-worklet-start.json` (`agrees: true`, 5 haps) | rule kept: a worklet source first runs in the first quantum with `q * 128 / 44100 > f32(begin)` |
| teardown | `measure-worklet-teardown` | `measure-worklet-teardown.json` (stop frame 13671, cut 13696 expected and observed) | rule kept: `ceil(stopFrame / 128) * 128` |
| V8 math | `v8-math` page kind | `v8-math-choice.json` | CORE-MATH ports for sin, cos, exp, expm1, log1p; V8's `cosh`; the host `pow` behind V8's special cases and the host `tanh` (as V8 does); JS `round` and `toInt32`; 100 % match per function |
| compressor warm-up | `compressor-fresh`, `compressor-warmup` | `compressor-warmup.json`, `engine-null-micro-native.json` | rule kept (outcome a, freeze after the quantum at 71808); the case's voice B gained `compressorRelease(1)` so the candidates differ |
| compressor libm | the four `compressor-*` cases | `compressor-libm.json` | rule kept: candidate 0 (platform `powf`/`log10f`) nulls at -inf on macOS arm64, nothing swapped |
| phaser | `phaser-notch`, `phaser-params` | `phaser-exp2.json` | rule changed: `DETUNE_EXP2_F32 = true` (the float `exp2f`; the double overload misses at -135 dB), every committed job byte-identical |
| duck | the six `duck-*` cases | `duck-first-ramp.json` | rule kept: the first fall starts at the pause frame (`setValueAtTime(1, pause)` inserted by the first ramp) |
| lfo | `mod-lfo-gain` | `mod-lfo-gain.json` | rule kept (outcome a): the shared worklet start rule nulls every case; `worklet.rs` unchanged |
| orbit-held `currentValue` | `mod-orbit-delaytime` | `mod-orbit-delaytime.json` | rule kept (outcome a): `Param::value()` |

## 6. Load variation of the superdough captures

Stage 13 found a one-off twin gate failure and a change to the voice-filter reference. The
follow-up used the same capture path under several loads. [Probe summary](load-variation.json)

- **voice-filter**: superdough's render is not bit-exact (-120 to -150 dB between renders; the
  previous voice's filter ring is cut by superdough's `onended` cleanup at a run-dependent quantum).
  No 0.01 dB RMS cell ever moved in 38 comparisons, but the channel peak, stored to 6 decimals,
  crossed a rounding boundary (0.429323 / 0.429324) in 5 of 31 renders. The dry run compares the
  envelope byte for byte, so it reports a change. It is a harness comparison issue, independent of
  load; the Rust fixture test is unaffected (peak tolerance 0.05 dB, the flip is 2e-5 dB).
  Follow-up: compare fixture envelopes with a tolerance in the dry run.
- **Rare twin failure:** further renders under different levels of CPU load and page concurrency
  did not reproduce it. The suspected cause was superdough cleanup or worklet timing on the
  main thread.
- **The same mechanism in a new fixture** (found, fixed): fx-native's first draft let the vowel's
  formant rings run past the voice's end; superdough's cleanup cut them at a run-dependent quantum
  (twin residual -44 to -71 dB, up to 0.02 absolute). The line now fades with `fxr(0.3)` (decision
  12's rule for voices that would ring past a main-thread cut) and its stem sits at the five-input
  summing floor (-146 dB).

The calibration ran at concurrency 4. Variation in twin residuals left the parity decision
unchanged across both runs.
