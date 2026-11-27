# 0130 engine decay bound: step 0 re-verification

At b7b18ce (origin/main). The claim was reviewed against 63b945d. Since then only a comment in
`schedule.ts` changed (`git diff 63b945d HEAD -- schedule.ts lookback.ts`: the `coerce.rs`
doc line). The call sites in `job-plan.ts` are unchanged apart from line shifts.

## 1. Where it lives and who calls it

| Item | Location | Callers |
|---|---|---|
| `FILTER_MAX_TAIL_SEC = 30` | `src/lib/render/schedule.ts:183` | `exportFilterTailSec` only |
| `exportFilterTailSec` (private) | `schedule.ts:189-257` | `exportTimingEvent` (`:346`) |
| `exportTimingEvent` | `schedule.ts:335-349` | `job-plan.ts:645` only |
| `exportDecay` | `src/lib/render/lookback.ts:147-185` | `job-plan.ts:648` only |
| bundled copy (generated) | `crates/strudeland-host/js/host.js:18015, 18385, 18870` | host `plan-render` |

- Both calls sit inside `planRender`'s `planPass` (`job-plan.ts:643-648`) and run only when
  `request.preset` is set (`tagged-loop`, `intro-loop`, `folded-loop`, `one-shot`).
- Preset requests come from `strudeland render --preset` (`crates/strudeland-cli/src/render.rs:42-50`)
  and from the desktop export's loop and one-shot modes (`src/lib/export/controller.ts:131` →
  `src-tauri/src/export_audio.rs:226`).
- They do not run for plain `strudeland render`, standard or stem exports (`preset: null`),
  `render:check` (superdough or Rust), the superdough goldens, `engine:refs` fixture jobs,
  `--planner batch`, `host:parity` (default and `--batch`), or `chunk` requests (`job-plan.ts:324`
  rejects chunk with preset). Those paths use `deriveLookback`/`lookbackSeconds` and `tailFrames = 0`.
- The one non-export runner is `host:parity --mixed-meter` (`scripts/render/host-parity.ts:149-177`),
  which plans the three loop presets on the render page and in the host. It compares the two
  TS copies with each other, not with the engine. It also pins `musicFrames` (771750), which
  does not depend on the tail.
- Conclusion: the code is preset-only, so "export-only" holds. No superdough path or golden
  depends on it.

## 2. What it computes and where the result goes

Inputs: the whole pass schedule `full.selected`, from `scheduleStart` to `toBar` (every hap of
the song for the opening pass, or the loop body for the repeat pass). Per hap it takes the raw
value, onset cycle, whole duration, cps and mixed-meter `timing`. It is not computed per chunk.

- `exportTimingEvent` returns `toEvent(..., engineTiming=true)`: engine coercions through
  `engineNum`/`roomNum`, with roomsize defaulting to 2. It adds `filterTailSec` (`exportFilterTailSec`)
  and `djFilter` (`djf` outside [0.49, 0.51] after `fround`).
- `exportFilterTailSec` sums these allowances:
  - LP/HP/BP: `max(0.5, 6·max(q,1/q)/edgeHz)` per stage, where `q = 10^(res/20)` for lp/hp and
    linear for bp. The edge is widened by the filter envelope (±octaves, `fanchor`) and by the
    built-in LFO (down to 30 Hz).
  - +1 s per `vowel`, and +0.5 s per phaser with `phaserdepth > 0`.
  - ×2 for 24 dB `ftype`.
  - The result is capped at 30 s. Any lfo/env/bmod that targets a filter, vowel or phaser
    control gives the full 30 s.
- `exportDecay`:
  - Per orbit it takes the maximum over the pass of `1.5·roomsize`, `min(1, delaytime)` and
    feedback.
  - Per hap: `life = own(release ≥ 0.01) + filterTailSec + bus feed + max(echo, reverb) + djf 0.5 s`.
    Here `echo = time + ceil(ln1000/ln(1/min(fb,0.98)))·(time + 128/44100)`.
  - It returns `lookbackSec = max life + 0.5` and `tailFrames = ceil((max(cut, tSec+life) − cut)·44100)`.

Where the result goes:

| Output | Path |
|---|---|
| chunk history (lead-in, chunk sizes, seam avoidance) | `decay.lookbackSec` → `history` (`job-plan.ts:649`) → `renderChunkPlans` (`:665-673`, events = `exportEvents`) → `windowPlan` c0 and `lookbackBars`/`scaleChunkBars` |
| last chunk's job `frames` | `tailFrames` → `planWindow(..., req.toBar === toBar ? tailFrames : 0)` (`:690`) → `windowPlan` `frames = end − start + tailFrames` (`plan.ts:153,167`) → `EngineJob.frames` |
| plan-render result | `export.tailFrames` and `export.repeat.tailFrames` (`job-plan.ts:741,746`; `crates/strudeland-host/src/planned.rs:50,62`) |
| buffer check | `checkBufferFrames(programFrames + tailFrames)` (`job-plan.ts:652`) |
| joiner | `pipeline_audio.rs:80,100,115,139` → `Joiner::for_plans(..., tail_frames)` (`join.rs:80,142`) |
| one-shot file length | `loop_export.rs:116-131` `trim_silent_post_roll` cuts to the last non-zero sample within the tail |
| loop files | `loop_export.rs:92,196` `fold_tail` sums the tail into the body (bytes, not length) |
| export summary | `summary.frames`/`seconds` (one-shot after trimming); `--info` carries the plan-render JSON |

## 3. Engine side

- Biquad: `MAX_TAIL_TIME = 30.0` and `MAX_TAIL_AMPLITUDE = 1/32768` (`crates/strudeland-engine/src/biquad.rs:28,30`).
  The exact Blink `TailTime` from the poles is at `biquad.rs:480-540` and feeds `propagates_silence` (`:629`).
- Reverb: IR `N = round(1.5·d·sr)`, at most 2^26 frames (`reverb.rs:12,80,269`). Orbit settings
  change only on a defined, different value (`reverb.rs:194`, `orbit.rs:446-463`).
- Delay: `MAX_FEEDBACK = 0.98` (`delay.rs:44-45`), feedback gain `min(|fb|, 0.995)`,
  `MAX_DELAY_TIME = 1.0` (`:37`), and each repeat adds 128 frames (`:15,40`). The feedback loop
  never ends by itself.
- Vowel: five bandpasses (`vowel.rs:26`). The voice stays alive until all five propagate silence
  (`vowel.rs:236`, `chain.rs:366-373`).
- Phaser: a notch at `center + 282`, Q `2 − min(depth·2, 1.9)` (`chain.rs:1790-1792`).
- Ladder: a worklet, disabled one quantum after its input goes (`chain.rs:470`), so it adds no tail.
- DJ filter: an orbit SVF (`djf.rs`) that never goes silent.
- Runtime silence detection already exists: a voice is freed once its sources stopped, every
  stage `is_done`, and `post < SILENCE_FLOOR = 1e-10` for one quantum (`voice.rs:61, 1945, 2083`).
  The ringing biquads are covered by that floor.
- Nothing in the engine computes a job-level or static decay bound. The binary has `render`,
  `signatures` and `controls` only. `unsupported_signatures` is at `capability.rs:683`.
- Job length: `EngineJob.frames` (`job.rs:30`) is set by the planner (`windowPlan` → `planEngineJob`)
  and only read by the engine (`render.rs:67`).
- A job carries what a port would need: per hap `t`, `duration`, `cycle`, `cps` and the value map
  (`job.rs:73-80`).

## 4. Is the friction real?

- No check ties the TS constants to the engine. The tests pin TS against TS only:
  - `tests/render-job-plan.test.ts:1530-1950`: exact `tailFrames` such as 7 s, 4 s and 1.5 s,
    and the filter cases at 1684-1741.
  - `crates/strudeland-host/tests/plan_render.rs:152-165` checks only that the tail is > 0 and
    sits in the last chunk's frames.
  - No vector, ref or golden covers a preset job. Neither `engine:refs` nor `render:check` sets a preset.
  - The finish golden is built from synthetic input (`crates/strudeland-render/src/characterization.rs:88-165`).
- One concrete divergence. `exportDecay` gives the DJ filter a flat 0.5 s (`lookback.ts:162`).
  The engine's SVF decays per sample by `m = 1 − 0.5^1.8·c` (`djf.rs:48-55,108-109`):

  | djf | cutoff | t60 | t90 (−90 dB) |
  |---|---|---|---|
  | 0.07 / 0.57 | 5.6 Hz | 0.68 s | 1.0 s |
  | 0.05 / 0.55 | 1.46 Hz | 2.6 s | 3.9 s |
  | 0.03 / 0.53 | 0.19 Hz | 20 s | 30 s |

  So the TS bound cuts the DJ filter's state early whenever djf is in [0.49−0.42, 0.57]. The
  energy left in that state is the signal's sub-10 Hz content, so the cut is probably far below
  hearing. It is still a cut, not a complete tail.
- Truncated by design:
  - Delay echoes are budgeted down to −60 dB (`lookback.ts:173`), but the engine's loop keeps
    sounding below that. A folded loop drops echoes under −60 dB.
  - Filter ringing is budgeted to about e^−19 against the engine's 1e−10 floor. This is below
    24-bit resolution.
- Checked against the engine and conservative:
  - Vowel: the largest `6·Q/f` over the 15 formant sets is 0.96 s (`ue`), and the real −90 dB
    time is at most 0.53 s.
  - LP/HP/BP: `6Q/f` is above the pole decay `ln(32768)·Q/(πf) ≈ 3.3Q/f`.
  - Phaser notch: about 0.1 s at most.
  - Ladder: the engine cuts it, but TS counts it as a biquad.
  - Orbit maxima over the whole song: the last chunk's orbit starts fresh, so TS over-counts here.
- Verdict: the friction is real, with no cross-check and one real under-estimate. Its
  consequences today are small and limited to presets.

## 5. Cost of the change as directed

The direction: the engine computes the tail from the jobs, the planner returns only the musical
window, and the pipeline extends the last job.

- Files touched: `schedule.ts` and `lookback.ts` are in `PLANNER_FILES`
  (`scripts/engine/provenance.ts:30-39`), and `job-plan.ts` changes too. That means
  `host:build`, `engine:refs --write` (a provenance-only hash change; fixture job bytes do not
  change because no fixture is a preset) and `host:parity --batch`/`--mixed-meter`.
- Engine jobs: only preset plans change. The last chunk's `frames` loses `tailFrames`, and
  `export.tailFrames` leaves `PlannedExport`, or is filled in by the pipeline. This breaks
  `plan_render.rs:152-165` and `render-job-plan.test.ts:1530-1950` (the tail assertions move to Rust).
- The history cannot move. `exportDecay.lookbackSec` decides the export chunks' lead-in, sizes and
  seams before any job exists. Moving only the tail leaves a second copy of the same life model
  in TS, which is worse than today. Making export history use `deriveLookback` (1.0·roomsize, no
  128-frame repeats, no filter or djf allowance) would shorten lead-ins and move chunk
  boundaries. That changes export bytes at seams.
- Moving the tail byte-identically needs all of these:
  - a bit-exact Rust port of `exportFilterTailSec` and `exportDecay`,
  - fed every chunk of the pass (the orbit maxima are taken over the whole pass, not the last job),
  - absolute times rebuilt as `job.t + chunk bufferStartSec`, which must round the same way as
    TS `tSec` before the `ceil`, or the tail shifts by one frame,
  - silence markers skipped exactly as in TS.
- A job-local bound (what "over EngineJob" means literally) differs from today whenever an earlier
  chunk set a larger orbit roomsize or delay. It is more truthful but not byte-identical.
- Unaffected:
  - superdough goldens and `render:check` (no presets),
  - the finish golden (synthetic input),
  - `render:vectors` (`joiner.json` comes from `assembleWindow` without presets),
  - the mixed-meter loop lengths (`pipeline.rs:592-640`), since loop files are body-length.

## 6. Visible behaviour

- One-shot: the file length is `musical + last non-zero sample ≤ tailFrames`. With any delay the
  engine is never exactly zero, so the length equals the bound. Any change to the bound changes
  the one-shot length and summary seconds.
- Loop presets: the length stays the body. The folded bytes change if the tail changes.
- Byte-identical is possible only with the exact whole-pass port described in section 5, plus a
  parity check of TS against Rust over the corpus for every preset. It then still keeps the TS
  history copy, so the refactor gains little.
- A dynamic "render until the engine is silent" bound would lengthen every one-shot that has a
  delay (whose loop never reaches exact zero), so it would need a cap and a level floor. That is
  a sound and length change.

## 7. New callers or copies since 63b945d

None. The only copy is the generated `host.js` bundle. No Rust copy of the bound exists in
`strudeland-render` or `strudeland-host`.

## Recommendation: re-scope

Do not move the bound into the engine. That would leave two copies of the life model, because
history has to stay in the planner. A byte-identical version needs a whole-pass port plus parity
for little gain, and a job-local version changes export bytes and one-shot lengths.

Re-scope to pinning the TS model to the engine. Scope:

1. Add an engine-side completeness test (Rust, in `strudeland-engine` or `strudeland-render`).
   For a table of single-hap preset cases (dry, room, delay fb 0, 0.5 and 0.98, lp/hp/bp with Q,
   24 dB, vowel, phaser, djf, a filter-targeting modulator), plan with the host (`plan-render`
   with `preset: 'one-shot'`) and render the last job longer than `tailFrames`. Then assert that
   the output after `tailFrames` stays below a stated floor (−90 dBFS peak, citing
   `MAX_TAIL_AMPLITUDE`). This fails when an engine DSP change outruns the TS bound.
2. Name the engine constants the bound cites in one place: `FILTER_MAX_TAIL_SEC`, the 1.5 IR
   factor, `DELAY_FEEDBACK_CEIL`, the 128-frame repeat and the vowel 1 s. A Rust test reads the
   TS values (as `v8-math`/`thresholds` provenance does, or through a small generated vector) and
   compares them with `biquad::MAX_TAIL_TIME`, `delay::MAX_FEEDBACK`, `delay::FEEDBACK_EXTRA_FRAMES` (currently `#[cfg(test)]`, `delay.rs:42`) and
   the reverb factor.
3. Fix the djf allowance if test 1 shows it (the decision below).

These steps change no planner file unless the djf decision changes the bound. The djf fix would
touch `lookback.ts`, so it would need `host:build`, `engine:refs --write` and `host:parity`.

## Decisions for the user

1. Re-scope to a cross-check instead of moving the code? Recommended: yes. If the move is still
   wanted, it must be the byte-identical whole-pass port, gated by a TS/Rust parity run.
2. DJ filter allowance (flat 0.5 s; the engine needs up to about 30 s at djf 0.03 or 0.53). The
   fix would lengthen one-shot exports and change folded-loop bytes for songs with djf ≤ 0.07
   or 0.53-0.57. Recommended: derive the allowance from the SVF pole, `ln(1000)/(−ln m)/sr`,
   capped at 30 s like the biquad, and accept the length change as an intended export change.
   Alternatively, keep 0.5 s and record it as a known difference if the test shows the residual
   is below the floor.
3. Delay echoes are truncated at −60 dB for loop folding. Recommended: keep it and document it
   as intended. Going to −90 dB would add about 50 % more post-roll to delayed songs and change
   one-shot lengths.
4. Floor for the completeness test. Recommended: −90 dBFS peak after `tailFrames`, from Blink's
   `MAX_TAIL_AMPLITUDE`. Delay is the exception, judged at −60 dB under decision 3.

## Decisions (2026-11-26)

- **Re-scoped.** The bound stays in TS. This task adds checks that pin it to the engine: a test comparing the TS constants with the engine's, and a completeness test that renders single-hap one-shot cases past `tailFrames`. No planner file changes, so no export changes.
- **DJ filter (user):** keep the 0.5 s allowance. The completeness test measures the leftover. If it is below the floor, the gap is written down as a known difference. If it is above, the test fails and the user decides again.
- **Delay:** the echo cut stays at -60 dB, documented (recommended).
- **Floor:** -90 dBFS, from the engine's `MAX_TAIL_AMPLITUDE`, with delay judged at -60 dB (recommended).
