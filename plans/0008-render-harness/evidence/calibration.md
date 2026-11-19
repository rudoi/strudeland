# Render gate calibration

This historical calibration preserves the aggregate measurements used to set the thresholds.
The archived reports retain aggregate decisions and reviewed synthetic fixture examples.

Probes ran on 2026-11-11 (UTC) on Playwright 1.64.1 and Chromium 156.0.8078.4.
Re-run all three after a Playwright bump or a Strudel sync:

These commands describe the calibration runs. Set `PROBE_EVIDENCE_DIR` to a
scratch output directory under the configured Cargo target for new reports.

```
bun scripts/render/probes/twin-null.probe.ts
bun scripts/render/probes/seam-stats.probe.ts
bun scripts/render/probes/mutation-sweep.probe.ts
```

Each probe ran over the whole corpus at concurrency 4, using the manifest windows at the time.
Wall times were 7.2 min for the twin, 5.1 min for the seam and 5.2 min for the mutation sweep.

`ceilTo(x, step)` (`scripts/render/probes/calibrate.ts`) rounds up to the step and never returns
less than one step, so a tolerance cannot calibrate to 0.

## 1. Twin null (`twin-null.json`)

Chunk 0 of every fixture rendered three times, each in its own page; worst of the three pairs.

| Constant | Before | After | Evidence field |
|---|---|---|---|
| `TWIN_BITEXACT_HOLDS` | `true` | `false` | `decision.TWIN_BITEXACT_HOLDS` |
| `TWIN_P95_DB` | 0.48 | 0.35 | `decision.TWIN_P95_DB` = ceilTo(1.25 x 0.263, 0.05) |
| `TWIN_MAX_DB` | 2.5 | 6.2 | `decision.TWIN_MAX_DB` = ceilTo(1.25 x 4.93, 0.1) |
| `TWIN_RESIDUAL_DB` | -10 | -18 | `decision.TWIN_RESIDUAL_DB` = ceil(-21.05 + 3) |
| `PROFILES.self.bandMeanDb` | 0.1 | 0.53 | `decision.self.bandMeanDb` = max(0.1, 2 x 0.263), rounded up |
| `PROFILES.self.bandP95Db` | 0.5 | 0.79 | `decision.self.bandP95Db` = max(0.5, 3 x 0.263), rounded up |
| `PROFILES.self.lufsDb` | 0.1 | 0.08 | `decision.self.lufsDb` = max(0.05, 2 x 0.04) |
| `PROFILES.self.peakDb` | 0.1 | 0.1 | `decision.self.peakDb` = floor (worst peak delta 0) |

`frameTolerance` 0, `maxOffsetSec` 0.01 and `hashMustMatch` false are unchanged (decision 11).

`PROFILES.engine` (Rust engine renders against these goldens) is calibrated separately, in
[`plans/0010-rust-engine-core/evidence/calibration.md`](../../0010-rust-engine-core/evidence/calibration.md).

Bit-equal (11): reverb-long, reverb-change, duck, odd-meter, bass-line, stems-shared,
silence-entry-perc, silence-entry-pad, noise-seed, song-linear, song-clip.

Twelve fixtures differed. Synthetic examples:

| Fixture | Residual dB | Worst band max dB | Kind |
|---|---|---|---|
| worklets | -26.71 | 1.13 (mid) | voice-level |
| pad-seam, stems-5, samples, clipping | -140 to -161 | ~0 | float rounding |

No corpus fixture is `stochastic`, so the bit-exact assumption (decision 4: noise, supersaw and
wavetable are seeded) does not hold in practice. Two kinds of difference:

- Float rounding (about -147 dB): the stage 9 finding for pad-seam and stems-5.
- Voice-level: one voice occasionally renders differently in one of several renders. Bisected
  by hand on worklets (stems mode, 8 renders): every render but one was bit-equal; the outlier
  differed only on the supersaw orbit, from the note at 15 s, at -9 dB. Each worklet line alone,
  and every pair of lines, renders bit-equal. Likely causes (not fixed here, no vendor patches in
  this plan): the supersaw seed reaches a pooled worklet node through `port.postMessage`
  (`synth.mjs:186`), which can arrive after the node starts processing, and `voiceSeed`'s
  occurrence counter in `rng.mjs` depends on the order superdough is called in.

The twin gate therefore runs in tolerance mode for every fixture, with thresholds set from these
outliers. They come from three renders per fixture; a rarer, larger flip can exceed them.

## 2. Seam statistics (`seam-stats.json`)

Real chunk plans of the 7 fixtures with `seam` (8 seams), one-sided head-quieter deficit per
`SEAM_BANDS` window, twin floor from a second render of the head.

| Fixture | Seam s | Worst dB (sub / mid / high) | Twin floor dB |
|---|---|---|---|
| pad-seam (2), reverb-long, reverb-change, song-clip | | 0 | 0 |

Pooled over 163 windows per band: sub p95 0.06, p99 0.97, max 2.48; mid p95 0.02, p99 0.79, max
1.11; high p95 0, p99 0.26, max 0.32.

| Constant | Before | After | Evidence field |
|---|---|---|---|
| `SEAM_TOLERANCE_DB` | 0.48 | 3.1 | `decision.SEAM_TOLERANCE_DB` = ceilTo(1.25 x 2.48, 0.05) |

`SEAM_TOLERANCE_DB` is now above `SEAM_FAIL_DB` (3), so the "windows over tolerance" warning of a
passing seam can only appear after escalation (fail line 3 + twin floor). That follows the rule;
the tolerance only labels windows, it decides nothing.

One repeatable head deficit had zero twin floor and remained below the seam fail line.

## 3. Mutation sweep (`mutation-sweep.json`)

The sweep covered 8 clean seams and 2 mutated seams, 107 clean orbit-bars and 3 dropped
orbit-bars, and 1374 clean bass notes and 3 dropped bass notes. False alarms count misses in
clean renders plus misses outside the dropped units in the mutated render. It picks the smallest
threshold with zero false alarms and recall of at least 0.9.

Seam (`seam.rows`; mutated pad-seam deficits 29.65 and 17.81 dB, twin floor 0):

| T dB | 0.25 | 0.5 | 1 | 2 | 3 | 4 | 6 |
|---|---|---|---|---|---|---|---|
| false alarms | 2 | 2 | 2 | 1 | 0 | 0 | 0 |
| recall | 1 | 1 | 1 | 1 | 1 | 1 | 1 |

Stems (`stems.rows`) had one false alarm at 6 dB and none from 8 to 24 dB. Recall was 1
throughout.

Bass (`bass.rows`):

| missDb | 6 | 8 | 10 | 12 | 14 | 16 | 18 | 20 | 22 | 24 |
|---|---|---|---|---|---|---|---|---|---|---|
| false alarms | 110 | 56 | 10 | 10 | 2 | 0 | 0 | 0 | 0 | 0 |
| recall | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 0.667 | 0.667 |

Dropped bass-line notes measure -20.54, -36.18 and -38.71 dB.

| Constant | Before | After | Evidence field |
|---|---|---|---|
| `SEAM_FAIL_DB` | 3 | 3 | `seam.choice` (unchanged) |
| `STEMS_MISS_DB` | 12 | 8 | `stems.choice` |
| `BASS_MISS_DB` | 12 | 16 | `bass.choice` |
| `BASS_CLUSTER_MIN` | 3 | 3 | `bass.rule.chosen.clusterMin` |
| `BASS_CLUSTER_SPAN_BARS` | 4 | 4 | `bass.rule.chosen.clusterSpanBars` |
| `BASS_CLUSTER_DEEP_DB` | 15 | 16 | `bass.rule.chosen.clusterDeepDb` |
| `BASS_FRACTION_MAX` | 0.2 | 0.2 | `bass.rule.chosen.fractionMax` |
| `BASS_FRACTION_MIN_NOTES` | 10 | 10 | not swept (kept) |
| `BASS_MIN_NOTES` | 5 | 5 | not swept (kept) |

Bass rule sweep at `missDb` 16: `clusterMin` 2-4, `clusterSpanBars` 2-8, `clusterDeepDb` 16-18,
`fractionMax` 0.1-0.3 (315 rules). A rule is valid when every clean bass fixture passes and
bass-dropped fails: 195 are valid. The chosen rule is the valid one closest to the start values
(distance in axis steps). Only `clusterDeepDb` moved, because the swept range starts at
`BASS_MISS_DB` (16) and the start value was 15. At 16 dB no clean fixture has a bass miss at all
and the phase-cancellation false alarms seen at 12 dB are gone. With
`BASS_CLUSTER_DEEP_DB = BASS_MISS_DB`, every miss can count toward a cluster.

## Gates removed from fixtures

No `seam` or `bass` gate was removed: every clean fixture passes both at the calibrated values
and every mutation is caught.

The mix gate was disabled for one slow-attack cue. Its opening model assumes a linear attack
without filtering, so the opening timing error exceeded `OPEN_LATE_TOL_SEC`. The synthetic
fixtures silence-entry-pad and silence-entry-perc exercise the opening rule.

## Goldens

All 23 fixtures eventually received new goldens. The slow-attack fixture needed a second run
after the mix gate was removed.

The first full `bun run render:check` then failed only on song-clip parity: peak -0.31 dB, sub /
low / mid p95 3.7 / 2.99 / 1.26 dB. Four more renders of song-clip, one at a time, all gave the same
deltas. So the golden, written at concurrency 4 under full load, had captured an outlier render:
the voice-level difference of section 1 (song-linear.js has a supersaw with `release(2)`). A
song-clip-only `--update-goldens` replaced it: `changed  frames 0  peak -0.31 dB  lufs -0.06 LU
bands max 11.19 dB  hash b9ccaf8c -> ed1ea9d9`. The next full run (all fixtures, mutations on, concurrency 4) was green: 23 pass, 3 mutations caught, 7 min 52 s, exit 0.

This is the main risk these thresholds leave open. The voice-level nondeterminism looks
load-dependent, and neither the twin tolerances nor `PROFILES.self` can absorb a 11 dB band flip.
A golden or a check run that catches an outlier render fails parity. The cause (section 1) needs a
fix in the render path, either a page-level patch or a recorded vendor patch, and that is outside
this plan's scope.
