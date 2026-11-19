# Rust engine calibration

This historical calibration preserves the aggregate measurements used to set the thresholds.
The archived reports retain aggregate decisions and reviewed synthetic fixture examples.

Probes ran on 2026-11-11 (UTC) with Playwright 1.64.1, Chromium 156.0.8078.4 and
`strudeland-engine 0.1.0` on macOS arm64. Re-run after a Playwright bump, a Strudel sync or a
change in engine residuals. The commands below record the historical checks.
Set `PROBE_EVIDENCE_DIR` to a scratch output directory under the configured
Cargo target when rerunning the probe.

```
bun scripts/render/probes/engine-null.probe.ts                       # engine-null.json (calibration)
PROBE_LABEL=final bun scripts/render/probes/engine-null.probe.ts     # engine-null-final.json (check)
cargo test -p strudeland-engine --test fixtures -- --nocapture       # check the archived envelopes
cargo test -p strudeland-engine --test micro -- --nocapture          # the micro residuals
```

`engine-null.probe.ts` renders every `rust: "supported"` corpus fixture (22) and every micro case
(59) with both engines at concurrency 4 (about 5 minutes).
The residual is `10*log10(sum((superdough - rust)^2) / sum(superdough^2))` over every sample of
every channel; there is no alignment search: both buffers start at frame 0 of the same request
plan. The run that set the profile wrote `engine-null.json`; the check run after the profile was
set in `parity.ts` wrote `engine-null-final.json`, whose `decision` is identical (section 2 lists
what moved between the runs).

## 1. Micro cases

All 59 cases meet their 05 section 5 target (-60 dB; delay and reverb -40 dB) by a wide margin and
their committed `maxResidualDb` in `crates/strudeland-engine/tests/refs/thresholds.json`. No
threshold changed in this calibration: each was set from evidence when its stage landed (measured
plus about 3 dB, or -140 dB for a bit-identical case; files `engine-null-micro-*.json`), and no
`interim` marker is left.

| Family | Cases | Residual dB | Target dB |
|---|---|---|---|
| adsr | 6 | -inf | -60 |
| delay | 6 | -inf | -40 |
| filter | 9 | -inf | -60 |
| fm | 3 | -inf | -60 |
| level | 2 | -inf | -60 |
| measure | 2 | -inf | -40 (feedback), -60 (suspend) |
| noise | 6 | -inf | -60 |
| orbit | 1 | -inf | -60 |
| osc | 11 | -inf | -60 |
| pan | 3 | -inf | -60 |
| param | 3 | -inf | -60 |
| pitch | 2 | -inf | -60 |
| reverb | 5 | -140.51 to -152.88 | -40 |

The reverb cases are not bit-identical because of the two Accelerate reductions the engine does
not reproduce (crate README, Known differences).

Before calibrating, the references were made deterministic (a dry `engine:refs` now reports "refs
are current" run after run): `noise-mix` (superdough's noise-mix teardown throws when such a voice
ends inside the render), six cases with three voices sounding at once at a note boundary
(Chromium's summing order of three inputs is not fixed) and `filter-ftype-numeric` (superdough's
disconnect cut a 24 dB ring at a varying quantum) were redesigned; all eight null at -inf
(`engine-null-micro-filters.json` `redesign`, `engine-null-micro-noise-fm.json`).

## 2. Fixtures

Every supported fixture passes every gate on the Rust render with the unchanged `thresholds.ts`
constants (`decision.gatesHold`; `stems-shared` stems is unverifiable as its manifest expects) and
parity against its superdough golden.

| Fixture | Residual dB | Worst 100 ms window dB (at s) | Superdough twin floor dB | Parity vs golden: peak / band mean / band p95 dB |
|---|---|---|---|---|
| pad-seam | -123.16 | -121.25 (1.1) | -148.06 | 0 / 0 / 0 |
| reverb-long | -95.38 | -93.47 (35.6) | -inf | 0 / 0 / 0 |
| reverb-change | -97.96 | -94.99 (30.2) | -inf | 0 / 0 / 0 |
| odd-meter | -776.33 | -734.54 (4.4) | -inf | 0 / 0 / 0 |
| bass-line | -inf | -inf | -inf | 0 / 0 / 0 |
| stems-5 | -147.63 | -145.54 (8.5) | -149.38 | 0 / 0 / 0 |
| stems-shared | -inf | -inf | -inf | 0 / 0 / 0 |
| silence-entry-perc | -inf | -inf | -inf | 0 / 0 / 0 |
| silence-entry-pad | -inf | -inf | -inf | 0 / 0 / 0 |
| clipping | -161.96 | -153.96 (6) | -162.37 | 0 / 0 / 0 |
| sine-basic | -154.40 | -146.09 (3.5) | -157.13 | 0 / 0 / 0 |
| voice-osc | -146.23 | -142.98 (12) | -145.36 | 0 / 0 / 0 |
| voice-mod | -151.34 | -146.12 (11.3) | -152.15 | 0 / 0 / 0 |
| voice-noise | -150.39 | -145.29 (11) | -149.75 | 0 / 0 / 0 |
| voice-filter | -135.11 | -119.22 (15) | -139.72 | 0 / 0 / 0 |
| voice-level | -148.30 | -144.01 (14.3) | -148.19 | 0 / 0 / 0 |
| fx-delay | -163.03 | -149.10 (16.2) | -163.22 | 0 / 0 / 0 |
| fx-room | -123.20 | -109.83 (13.3) | -155.15 | 0 / 0 / 0 |

Numbers from `engine-null-final.json`. LUFS deltas are 0 everywhere, frame deltas 0, alignment
offsets 0. The superdough twin floor is two superdough renders of chunk 0 (`-inf`: bit-equal).
Between the two runs the residuals and twin floors that depend on Chromium's summing order or on
superdough's disconnect timing varied (voice-filter -145.09 then -135.11 dB); every parity delta, gate verdict and micro residual was identical,
and so was the decision.

What the numbers say:

- Against the superdough render of the same run (`parityVsSuperdough`), every fixture's band,
  LUFS and offset deltas were 0. Some differences against goldens tracked the same drift
  measured between superdough and the golden (`goldenDrift`).
- Most targeted fixtures sit at their superdough twin floor (-135 to -166 dB: the summing order
  of three or more voices, which Chromium itself does not fix).
- The reverb fixtures (reverb-long, reverb-change, fx-room) sit at
  -95 to -125 dB: the IR normalization and the direct convolver stage are Accelerate reductions the
  engine sums in a fixed order (about 1e-6 relative on the wet path, more for reverb-long's 30 s
  IR). odd-meter differs only by subnormal values (-776 dB).
- Superdough can disconnect a voice before its filter tail has settled. The engine keeps the
  tail until Blink's silence propagation ends it. The change affected peak by 0.01 dB in the
  original measurement. [Engine behavior and known differences](../../../crates/strudeland-engine/README.md)

## 3. `PROFILES.engine`

`engineDecision` (`scripts/render/probes/calibrate.ts`) takes the worst delta of every supported
fixture against its golden, or the superdough twin floor's, whichever is larger, and applies the
`self` profile's margins and floors (no engine tolerance below what superdough needs against its
own goldens). `decision.engine` in `engine-null-final.json`:

| Field | Start (copy of self) | Calibrated | Worst measured | Rule |
|---|---|---|---|---|
| `frameTolerance` | 0 | 0 | 0 | worst frame delta |
| `maxOffsetSec` | 0.01 | 0.01 | 0 | max(0.01, 2 x worst) |
| `peakDb` | 0.1 | 0.1 | 0.01 | max(0.1, 2 x worst) |
| `lufsDb` | 0.05 | 0.05 | 0 | max(0.05, 2 x worst) |
| `bandMeanDb` | 0.1 | 0.1 | 0.02 | max(0.1, 2 x worst) |
| `bandP95Db` | 0.5 | 0.5 | 0.13 | max(0.5, 3 x worst) |

`hashMustMatch` stays false (the engine's PCM differs from superdough's in the last bits). The
profile equals `self` because the engine is as close to superdough as superdough is to itself; it
is now calibrated rather than provisional, and every field in `src/lib/render/parity.ts` cites
`engine-null-final.json`.

## 4. Gate constants on Rust renders

The probe ran each fixture's gates on the Rust renders with the `thresholds.ts` constants: all pass
(twin exact, seam, stems, bass, mix), so no constant changes. `render:check --engine rust` also
catches the three mutations.

## 5. Fixture envelope guard (`fixtures.rs`)

`engine-fixtures-envelope.json`: over the 24 supported synthetic fixture chunk jobs the worst
window mean is 0.003 dB, p95 0.005 dB and peak 0.000 dB, which is the 0.01 dB rounding of the
stored superdough envelope. The `fixtures` tolerances of `thresholds.json` tightened from the
placeholders 1.0 / 3.0 / 1.0 dB to window mean 0.05 dB, p95 0.1 dB, peak 0.05 dB (floor -60 dBFS
unchanged); the `placeholder` marker is gone.

## 6. Plan corrections

The calibration confirmed the measured rules and the interpretation of Blink's behavior. The
noise-mix teardown error originates in superdough.
