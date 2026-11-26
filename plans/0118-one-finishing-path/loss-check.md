# 0118 loss check (refine pass)

Compares the refined plan with the original plan commit (2ee5154), the user's four decisions,
and the refine findings.

## The user's decisions

| Decision | Status |
|---|---|
| 1. All bytes stay identical; the MP3 flush is explicit on the encode request (`NoGap` standard, `Gap` one-shot and loops); #146 checks the standard tails | Preserved. The refine adds that `NoGap` with a loop range is an `Encode` error, which no caller can reach, so no byte moves. |
| 2. Both leveling strategies stay, behind one loop with an explicit, tested switch | Preserved. |
| 3. `AGENTS.md`'s true-peak rule states both correction rules; "never loosen" stays | Preserved (stage 006). |
| 4. A permanent characterization test, refreshed only on encoder bumps, unchanged after every stage | Preserved and widened: more cases (wrap peak, program cap, unmeasurable body, silent intro-loop) and an error section. It is still written only after an encoder bump, and now refuses to write off macOS aarch64. |

## The original plan

**Preserved:**

- the six stages, their order, the commit messages and the trailer;
- every frozen surface;
- the golden schema and its case names;
- the planning split, the encode request, the splice and the finishing loop's shape;
- the per-strategy warnings and summaries;
- the byte-identity arguments, including the Limited `level_ms` base of 0;
- the "replace, don't layer" rule and the test cadence.

**Modified:**

- **Progress (Decision 6).** The original moved the loop presets' `Encoded{pass}` one decode
  earlier. That was a visible change on a verify-decode error. Now each strategy keeps today's
  emit point: Limited emits after the encode and before the decode, and RepeatSafeStatic emits
  after the whole pass, before the correction check. The proposal has the full event map. Stage
  005 step 2.3 is replaced, `progress_precedes_the_last_decode` is Limited-only, and
  `static_progress_follows_the_whole_pass` is added.
- **Loop fixtures.** They run at cps 2.0, so a 4-bar window is 2 s instead of 8 s. The fixtures
  now set the window, cps and start bar explicitly over `testing::audio`'s `0..1` window.
- **Test-runtime risk.** The text claimed the crate builds at opt-level 3 in dev. That is wrong:
  only dependencies do. The 30 s measure-and-shorten rule stays.
- **`FileCut`** stays defined in `splice.rs`, and `finish.rs` imports it. The original moved it
  in stage 005.
- **Stage 004's error-order list** now matches `loop_export.rs:74-107`: two conversions, then
  the combined size, then `copy_opening` or the history check.
- **Stage 002** states the pure cut and paste and the two substitutions (the repeat pass passes
  `None`). Expected values are read from the code before the move. The per-hap error type is
  `strudeland_engine::RenderError`.
- **Stage 003** lists every call site. `tests/encode.rs:834` is added to the findings' list,
  since it calls `encode_once` too. It also records the deliberate crate pub-API removals and
  changes.
- **Stage 006** has the exact README and `AGENTS.md` locations. It also moves the correction
  loop out of `encode/` in both the README steps and the Layout.
- **reverify.md** has the corrected line references.

**Dropped:**

- the "Progress order and timing" risk, since there is no longer any progress change;
- the `pub(crate)` instruction for `PipelineAudio` and `LoopAudio`, since descendant modules
  already see crate-root private items.

## Refine findings

| # | Finding | Applied in |
|---|---|---|
| 1 | Progress per strategy, event map, Static progress tests | proposal (sequence, table, Decision 6, Risks, Tests), 005, reverify |
| 2 | `.prettierignore` gets `tests/golden/` | proposal, 001 (step 4, validation), 000 (lint once) |
| 3 | cps 2.0 loop fixtures; the crate is unoptimised in dev | proposal (matrix, Risks), 001 |
| 4 | Explicit fixture overrides for the loop cases | proposal, 001 |
| 5 | Every encode call site; pub-API removals stated | proposal (Encode request), 003, reverify |
| 6 | Wrap-peak, unmeasurable-body and silent intro-loop cases, plus the cap guards | proposal, 001 |
| 7 | Error section (Ogg quality 99; NaN) recorded as `{error, progress}` | proposal, 001 |
| 8 | Stage 002 expected values, pure move, `None` for the repeat | proposal, 002 |
| nice | Drop `pub(crate)`; the write variable refuses off macOS aarch64; the jsmath precedent is stricter here, CI enforces it, and a `--release` run; the `Cargo.lock` edge; licence pins and sha2's licence; the stage 006 doc locations; `FileCut` in `splice.rs`; `NoGap` plus a range is an error; the MP3 warning equivalence; the 004 error order; the engine error type; the `level_ms` note | proposal, 001-006 |

None was left out. For the wrap-peak case, the program's handoff mirrors the wrap through the
10 ms blend, so the wrap dominates only by the blend's residue. The plan therefore tells the
fixture to oppose the opening's last 10 ms to the repeat's tail, and the guard checks it from
the summaries alone. If the guard still fails, the implementer adjusts the fixture, never the
guard.
