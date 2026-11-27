# 001 pin the export decay bound to the engine

Read [000-prompt.md](000-prompt.md) first. Its contract, rules, gate and STOP list apply here.

## Goal

Add `crates/strudeland-render/tests/decay_bound.rs`:

- a parity test showing that the TS post-roll bound uses the engine's constants;
- a completeness test showing that, for one hap of every control family, the engine's output
  after `tail_frames` is under the floor.

Expose the engine constants through `strudeland_engine::decay`. Document the bound in the
render README. Change no planner file, no engine behaviour and no generated file.

## Prerequisites

- `git diff --name-only b7b18ce HEAD` lists only `plans/`.
- The V8 download and cmake are available, since `strudeland-render` tests build the host and
  the encoders.

## Files

| File | Change |
|---|---|
| `crates/strudeland-engine/src/lib.rs` | add `pub mod decay` (re-exports only, with a doc comment naming `exportFilterTailSec`/`exportDecay` and the test) |
| `crates/strudeland-engine/src/delay.rs` | drop `#[cfg(test)]` from `FEEDBACK_EXTRA_FRAMES` (`:42`) |
| `crates/strudeland-engine/src/reverb.rs` | `pub const IR_DURATION_FACTOR: f64 = 1.5;` beside the other constants (`:74-89`), used at `:269` (`d * IR_DURATION_FACTOR`) |
| `crates/strudeland-render/tests/decay_bound.rs` | new |
| `crates/strudeland-render/README.md` | new subsection under "Game audio presets" |
| `AGENTS.md` | one sentence in the render pipeline rule |

The `decay` module:

```rust
pub mod decay {
    pub use crate::biquad::{MAX_TAIL_AMPLITUDE, MAX_TAIL_TIME};
    pub use crate::delay::{FEEDBACK_EXTRA_FRAMES, MAX_DELAY_TIME, MAX_FEEDBACK};
    pub use crate::reverb::IR_DURATION_FACTOR;
    pub use crate::vowel::FORMANTS;
}
```

If a re-export is refused because an item is `pub(crate)`, make that item `pub`. That is still
a visibility change only.

## Steps

1. **Engine exposure.** Make the three engine edits above. Run
   `cargo test -p strudeland-engine --lib` to confirm that the reverb unit tests (`:1327`, IR
   length) still pass.

2. **Harness** (in `decay_bound.rs`, with no shared `common` dependency needed):
   - `SR = strudeland_engine::SAMPLE_RATE as f64`.
   - `song(voice: &str) -> String` returns `"const meta = { bpm: 120 }\n$: " + voice`. The
     song is one bar of 2 s at 4/4, and the pattern gives exactly one hap per bar. Strings
     inside pattern code follow the corpus spelling: mini-notation in double quotes, plain
     strings such as `ftype('24db')` in single quotes.
   - `plan(host, code) -> PlannedRender` calls `host.plan_render` with
     `LengthRequest::Length { bars: 1 }` and `preset: Some(ExportPreset::OneShot)`, with every
     other request field `None` or default, as `plan_request` does in
     `crates/strudeland-host/tests/plan_render.rs:16-25`. Assert `errors` is empty and
     `chunks.len() == 1`, then return the plan.
   - `tail(host, voice) -> u64` returns `plan(...).export.unwrap().tail_frames`.
   - `render_past(job_text, extra) -> (Vec<Vec<f32>>, u64)`:
     1. `EngineJob::from_json`, and assert
        `strudeland_engine::capability::unsupported_signatures(&job)` is empty (the call
        `strudeland-render`'s `plan.rs:53` makes);
     2. render it, then render a clone with `frames += extra`;
     3. assert both have no `errors`, and assert the first `frames` of every channel are
        bit-equal (`to_bits`);
     4. return the long render and the original `frames`.

     A refused case is a STOP.
   - `peak(channels, range)` returns the maximum `|x|` over both channels, and `dbfs(x)` returns
     `20·log10(x)`.

3. **Parity test** `the_bound_uses_the_engine_constants`. Plan each voice with `tail`. For
   cases 2 onward, `ΔF = tail(case) − tail(dry)`. Assert `|ΔF − expected| ≤ 1` frame. Collect
   every case's line (case, ΔF, expected), print the table with `eprintln!`, then fail once
   with all mismatches.

   The base voice `D` is `s("sine").note(48).release(1)`.

   | # | Voice | Expected |
   |---|---|---|
   | 1 | `D` (absolute `tail`, not ΔF) | `1.0·SR` (pins the TS sample rate and release) |
   | 2 | `D.lpf(400).lfo({ c: 'cutoff', r: 1, dep: 0.5 })` | `MAX_TAIL_TIME·SR` |
   | 3 | `D.bpf(1).bpq(10000)` | `MAX_TAIL_TIME·SR` (the static cap) |
   | 4 | `D.room(1).roomsize(0.5)` | `IR_DURATION_FACTOR·0.5·SR` |
   | 5 | `D.room(1).roomsize(4)` | `IR_DURATION_FACTOR·4·SR` |
   | 6 | `D.delay(1).delaytime(0.5).delayfeedback(0)` | `echo(0.5, 0)` |
   | 7 | `D.delay(1).delaytime(0.5).delayfeedback(0.5)` | `echo(0.5, 0.5)` |
   | 8 | `D.delay(1).delaytime(0.1).delayfeedback(0.98)` | `echo(0.1, 0.98)` |
   | 9 | `D.delay(1).delaytime(0.1).delayfeedback(0.99)` | `echo(0.1, 0.99)`, equal to case 8 through the ceiling |
   | 10 | `D.delay(1).delaytime(1.5).delayfeedback(0.5)` | `echo(1.5, 0.5)` (time clamped) |
   | 11 | `D.vowel("a")` | `1.0·SR` |

   The delay formula:
   - `echo(t, fb) = (tc + R·(tc + FEEDBACK_EXTRA_FRAMES as f64 / SR))·SR`
   - `tc = t.min(MAX_DELAY_TIME)`
   - `R = 0` when `fb ≤ 0`, else `ceil(ln(1000) / ln(1 / fb.min(MAX_FEEDBACK)))`

   The `ln(1000)` is the −60 dB cut, a TS decision (decision 3) and not an engine constant.
   Name it in a comment.

   Also assert the vowel allowance against the engine's table: the maximum over `FORMANTS`
   `(letter, freqs, _, qs)` of `6·qs[i]/freqs[i]` is at most 1.0 s (reverify §4: 0.96 s, `ue`).
   Print the maximum and its letter.

   Spelling check (not a STOP): if case 2 does not reach the cap, first confirm the modulator
   spelling against `tests/render/corpus/fx-mod.js:9` (`c:` is the control key) and
   `modConfigs` in `schedule.ts`. If the spelling is right and ΔF still differs, that is a
   parity mismatch: STOP.

4. **Completeness test** `one_shots_decay_below_the_floor_after_the_bound`. `EXTRA = 5·SR`
   frames. For each case:
   - plan it, take the job, and `render_past(job, EXTRA)`;
   - `after = peak(long, frames..frames+EXTRA)`;
   - the floor is `MAX_TAIL_AMPLITUDE`, except for the delay cases, where it is
     `1e-3 · peak(long, 0..frames)`.

   Evaluate every case, print one table row per case (name, `tail_frames`, `after` in dBFS, the
   floor in dBFS, pass/fail), then fail once with the list of failing cases. This way one run
   gives every number. The base voice `B` is `note(36).s("sawtooth").gain(1)`.

   | Family | Cases |
   |---|---|
   | envelope release | `B.release(0.01)`, `B.release(0.5)`, `B.release(4)` |
   | lowpass | `B.lpf(1500).lpq(15)`, `B.lpf(40).lpq(30)` |
   | highpass | `B.hpf(1500).hpq(15)`, `B.hpf(20000).hpq(30)` |
   | bandpass | `B.bpf(1500).bpq(10)`, `B.bpf(40).bpq(20)` |
   | 24 dB cascade | `B.lpf(40).lpq(30).ftype('24db')`, `B.bpf(40).bpq(20).ftype('24db')` |
   | filter envelope | `B.lpf(300).lpq(20).lpenv(-4)` |
   | filter modulator (30 s cap) | `B.lpf(400).lpq(20).lfo({ c: 'cutoff', r: 1, dep: 0.5 })` |
   | ladder | `B.lpf(200).lpq(20).ftype('ladder')` |
   | vowel | `B.vowel("a")`, `B.vowel("ue")` |
   | phaser | `B.phaser(1)`, `B.phaser(1).phaserdepth(1)` |
   | DJ filter | `djf` 0.03, 0.05, 0.53 and 0.55, each on `B` and on `s("sine").note(60).gain(1)` (8 cases) |
   | reverb | `B.room(1).roomsize(0.5)`, `B.room(1)` (default 2), `B.room(1).roomsize(10)` |
   | delay (−60 dB) | `B.delay(1).delaytime(0.1).delayfeedback(0.98)`, `B.delay(1).delaytime(0.1).delayfeedback(0.99)`, `B.delay(1).delaytime(0.5).delayfeedback(0.5)`, `B.delay(1).delaytime(1.5).delayfeedback(0.9)` |

   If a spelling does not reach the intended control (for example the ladder or phaser name),
   check it against the corpus and `crates/strudeland-engine/controls.md`, and fix the
   spelling. A case that is right and still refused is a STOP.

5. **Run the targeted test** (gate step 1). It is green, or it STOPs: any failing case goes to
   the user with the printed table, unaltered.

6. **README.** Add `### Post-roll bound` under "Game audio presets" in
   `crates/strudeland-render/README.md`. Short prose covering these points:
   - The planner (`exportDecay` and `exportFilterTailSec`, `src/lib/render/`) bounds the tail a
     preset renders after the arrangement. The budget is release, filter ringing (capped at
     30 s, the engine's biquad tail time), reverb at 1.5 times roomsize (the IR length), and the
     delay's first tap plus the repeats down to −60 dB (one render quantum per repeat, feedback
     at most 0.98, delay time at most 1 s).
   - **The delay cut is intended.** Echoes below −60 dB are dropped from one-shots and are not
     folded into loops. Going to −90 dB would add about 50 % post-roll to delayed songs
     (reverify, decision 3).
   - Filter rings are budgeted to about e^−19, below 24-bit resolution.
   - **Known difference, the DJ filter.** The planner allows a flat 0.5 s. The engine's SVF at
     djf 0.03/0.53 needs up to about 30 s to fall 90 dB (reverify §4 table). State the measured
     leftover after the bound for each tested djf value and source, against the −90.3 dBFS
     floor.
   - `tests/decay_bound.rs` pins the constants to the engine (`strudeland_engine::decay`) and
     renders a one-shot per control family past the bound. Its floors are never loosened to
     absorb a defect.

7. **AGENTS.md.** Add one sentence to the render pipeline rule: the preset post-roll bound is
   pinned to the engine by `crates/strudeland-render/tests/decay_bound.rs` (constant parity
   through `strudeland_engine::decay`, and one-shots rendered past the bound), and its floors
   are never loosened to absorb a defect.

8. `cargo fmt --all`, then the full gate in [000-prompt.md](000-prompt.md), once, in order.

## Validation

- Every gate step has passed, as stated in 000.
- The measured table, the parity ΔF lines and the djf README numbers are in the stage report.
- `git diff b7b18ce -- crates/strudeland-engine/src` shows only the decay module, the removed
  `#[cfg(test)]` and the named reverb factor.

## Commit message

```
test(render): pin the export decay bound to the engine

Compare the post-roll bound's constants, read from host one-shot plans,
with the engine's biquad tail time, reverb IR factor, delay feedback
ceiling, repeat quantum and delay maximum, and the vowel allowance with
the formant table. Render a one-shot per control family past the bound
and check what is left against Blink's tail amplitude (delay at -60 dB).
Document the bound, the intended delay cut and the DJ filter difference.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
