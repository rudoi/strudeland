# 0130 export decay bound: pin it to the engine

Evidence and the binding decisions: [reverify.md](reverify.md) (at `b7b18ce`), Decisions
(2026-11-26).

## Goal

Keep the export post-roll bound in TypeScript, where it is today
(`src/lib/render/schedule.ts` `FILTER_MAX_TAIL_SEC`, `exportFilterTailSec`, `exportTimingEvent`;
`src/lib/render/lookback.ts` `exportDecay`). Add the cross-check it lacks (reverify §4), so that an
engine change that outruns the bound, or a TS constant that drifts from the engine's, fails a test.

## Scope

In:

1. **Constant parity.** One Rust test compares the values the TS bound uses with the engine
   constants they mirror:

   | TS (as the bound uses it) | Engine |
   |---|---|
   | `FILTER_MAX_TAIL_SEC = 30` (the per-voice filter cap) | `biquad::MAX_TAIL_TIME` |
   | `1.5 · roomsize` (the reverb IR) | the reverb IR factor (`reverb.rs:269`, `d * 1.5`) |
   | `DELAY_FEEDBACK_CEIL = 0.98` | `delay::MAX_FEEDBACK` |
   | `128 / SAMPLE_RATE` per feedback repeat | `delay::FEEDBACK_EXTRA_FRAMES` (today `#[cfg(test)]`) |
   | `min(1, delaytime)` | `delay::MAX_DELAY_TIME` |
   | `+1 s` per vowel | the largest `6·Q/f` over `vowel::FORMANTS` (0.96 s, `ue`) must not exceed it |
   | `SAMPLE_RATE = 44100` (tail frames) | `SAMPLE_RATE` |

2. **Completeness.** 33 single-hap one-shot cases per control family (beside the 11 parity
   cases), planned through the host (`plan-render` with `preset: one-shot`), rendered with the
   engine past `tailFrames`. The peak after the bound must be at most `MAX_TAIL_AMPLITUDE`
   (1/32768, −90.3 dBFS). Delay is judged at −60 dB relative to the render's peak before the
   bound (decision 3). The DJ filter cases are judged against their own floor of −70 dBFS (the
   user's re-decision, reverify Decisions: the measured leftover is −76.9 dBFS at most, so
   about 6 dB of margin still catches a regression).
3. **Documentation.** The bound, its −60 dB delay cut with the measured margins, and the DJ
   filter's flat 0.5 s as a known difference (the measured leftover and need, at the measured
   source level) are written down in `crates/strudeland-render/README.md`.

Out:

- Moving the bound into the engine (re-scoped away, reverify §5 and Decisions).
- Any change to a `PLANNER_FILES` file (`scripts/engine/provenance.ts:30-39`), engine DSP,
  job, golden, ref, vector, `host.js` or export output. So no `host:build`, `engine:refs --write`
  or `host:parity` obligations arise.
- Changing the djf allowance. The user kept it (reverify Decisions, re-decided): exports do
  not change. A djf case above its −70 dBFS floor still stops the task.

## Design

### Where the checks run

One new integration test, `crates/strudeland-render/tests/decay_bound.rs`. `strudeland-render`
already depends on `strudeland-host` and `strudeland-engine`. Its tests already plan through
the host (`tests/pipeline.rs`). So both checks run under `cargo test --workspace` on every CI
workflow (`macos.yml:62`, `native.yml:87`), with no Chromium and no Bun.

### How each side's values are read (the mechanism)

- **Engine side:** a `pub mod decay` in `crates/strudeland-engine/src/lib.rs` re-exports the
  constants in the table above. Its modules are `pub(crate)` today. Two source edits beyond
  visibility, neither of which changes behaviour:
  - `FEEDBACK_EXTRA_FRAMES` loses `#[cfg(test)]`.
  - The reverb factor gets a name, `pub const IR_DURATION_FACTOR: f64 = 1.5`, used at its one
    site (`d * IR_DURATION_FACTOR`, the same f64 product).
- **TS side, read from its behaviour:** the test plans one-shot cases with the host (the
  compiled `host.js`, which is what exports run) and reads `export.tail_frames`. Differences
  against a dry case isolate each constant: `ΔF = tail(case) − tail(dry)`, compared with the
  value from the engine constants within ±1 frame (the `ceil` of a sum of floats).

Why this mechanism and not the alternatives:

- **Reading the TS source with a regex** would pin spelling, not behaviour. The reverb `1.5` and
  the `128` are inline literals in `lookback.ts`, a planner file this task may not edit to name
  them.
- **`controls --json` read from a `bun test`** would make `bun test` build and run cargo, which it
  never does today. The dump is documented as read by nothing (`capability.rs:720`).
- **A new `engine:vectors` file** adds a generated file and its staleness rule, and vectors are
  frozen in this task.
- The chosen check reads the bundle exports actually run, so a TS edit that skipped `host:build`
  is caught by the existing `bun test` staleness test, not missed here. It needs no generated
  file, and it shares its harness with the completeness test.

### Completeness harness

Per case:

1. Write a one-bar song (`const meta = { bpm: 120 }` and one `s(...)` voice with the family's
   controls).
2. `Host::plan_render` with `LengthRequest::Length { bars: 1 }` and `preset: OneShot`. Assert one
   chunk, no errors, and `capability::unsupported_signatures` empty. A refused case is a STOP, not a dropped
   case.
3. Parse the last job. Render it as planned, then render it again with `frames + EXTRA`. Assert
   that the first `frames` are bit-equal, so the extension does not change the bounded part.
4. Measure the peak over both channels in `[frames, frames + EXTRA)` and compare it with the
   floor.

`EXTRA` is 5 s: more than the longest single delay repeat (`MAX_DELAY_TIME` plus 128 frames)
several times over. The test prints a table (case, `tail_frames`, peak after, dBFS) for the
stage report and the README.

## Risks

- **A case leaks above the floor.** That is the finding the test exists for, and the task
  STOPs. The prototype measured every case: only the sawtooth djf cases sit above −90.3 dBFS
  (−76.9 and −76.3 dBFS), which the user accepted with the −70 dBFS djf floor. The tightest
  delay margin is 1.5 dB (time 1.5, feedback 0.9). The reverb tail is below −239 dBFS.
- **Runtime.** The longest cases are the 30 s modulator cap and the high-feedback delay; keep
  `delaytime` at 0.1 for those. The engine is built at opt-level 3 in dev (`Cargo.toml:5-9`).
- **Cross-platform levels.** `jsmath` `pow`/`tanh` use the host libm, but floor assertions
  have margin, and no byte comparison is made.

## Open questions

None blocking. Decision 3 does not name the delay's −60 dB reference. The plan uses the render's
own peak before the bound. The TS model budgets from the first tap, and the cases use
`delay(1)`, where the first tap is as loud as the dry voice, so the two references agree within
the overlap of dry voice and tap.
