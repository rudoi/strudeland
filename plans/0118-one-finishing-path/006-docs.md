# 006: docs

## Goal

The crate README, the crate doc and `AGENTS.md` describe one finishing loop with two leveling
strategies, and state both correction rules. No code changes.

## Prerequisites

- **Stage 005 is committed,** and the characterization passes.
- **`GLOSSARY.md`** already has **Finishing**, **Leveling strategy**, **File cut** and **Loop
  splice** (from the plan commit). Re-read them, and fix any entry the code contradicts.

## Implementation

1. **`crates/strudeland-render/README.md`:**
   - **"The lossy correction loop" (`:122-131` at the plan commit):** retitle it "Finishing and
     the lossy correction loop". It should say that every render finishes through one loop,
     with one of two leveling strategies:
     - **Limited** (plain renders and one-shots): the static gain to the target, then the
       true-peak limiter. A decoded overshoot lowers the limiter's ceiling by the overshoot
       plus 0.1 dB, and the gain is unchanged.
     - **Repeat-safe static** (loop presets): one static gain shared by the files, capped for
       the handoff and wrap, with no limiter. The worst decoded overshoot lowers the shared
       gain by the overshoot plus 0.1 dB, and every file is re-encoded.

     Both strategies correct at most twice, then warn. Loudness off never corrects. Keep the
     `limiterCeilingDbtp` sentence, and add that a loop preset reports it as `null`.
   - **The loop sentence at `:107-110`:** point it at that section instead of restating it.
   - **The MP3 flush:** plain renders end the LAME stream without the gap flush; one-shots and
     loop files use the gap flush, so a gapless decoder recovers the whole span. Name #146 only
     as "a pending check of plain-render MP3 tails" (no task id in the README).
   - **The characterization golden:** under "Determinism", add `tests/golden/finish.json`,
     what it pins, the reference platform, and that it is refreshed only after an encoder crate
     bump.
2. **`crates/strudeland-render/src/lib.rs`:** the module doc's step 5/6 line links
   `finish::finish_files` through prose, since it is private. "Level and encode" becomes
   "Finish: level, encode, verify and correct each file cut ([`finish`], [`render_export`])".
3. **`AGENTS.md`:**
   - **Rules, render pipeline:** replace "A lossy file is decoded again and its true peak
     corrected by lowering the limiter ceiling (at most twice, then a warning); never loosen that
     to absorb a defect." with:

     "A lossy file is decoded again and its true peak corrected, at most twice and then a
     warning: a limited render (plain or one-shot) lowers its limiter ceiling, and a loop preset
     lowers the static gain its files share. Never loosen either to absorb a defect."
   - **Layout, the render pipeline bullet:** add these after `join`:
     - `finish` (the one finishing loop: the `Leveling` switch between Limited and
       RepeatSafeStatic, through the `Codec` seam);
     - `splice` (the loop splice and its file cuts).

     Next to `export`, add `loop_export` (the presets' public types and `finish_export`). In
     `encode/`, add `EncodeRequest` with the explicit MP3 flush.
   - **Layout:** mention `render_planned` beside `plan` ("`plan` (host to jobs, refusal of
     unsupported material; the only V8 caller) and `render_planned` (a plan to joined audio
     through an injected chunk renderer)").
4. **Check** that nothing else names deleted items:

   ```sh
   rg -n "encode::encode\b|encode_with|finish_loop|encode_mp3_with_loop|encode_once_with_loop|Real::Legacy" --glob '!plans/**'
   ```

## Tests

None. These are docs only. `crates/strudeland-cli` `SKILL.md` is untouched (there is no CLI
change), and its mention test still passes.

## Validation

- **The stage gates,** logged once: fmt check, clippy and `cargo test --workspace`.
  `characterization` passes, and the golden is untouched.
- **`git diff --stat`** shows only `AGENTS.md`, `crates/strudeland-render/README.md`,
  `crates/strudeland-render/src/lib.rs` and, if a fix was needed, `GLOSSARY.md`.

## Commit message

```
docs(render): describe the one finishing loop and both correction rules

The README and the agent guide now say that every render finishes
through one loop with two leveling strategies: a limited render lowers
its limiter ceiling on a lossy overshoot, and a loop preset lowers the
static gain its files share. The layout names the finishing loop, the
loop splice and render_planned, and the README documents the MP3 flush
per path and the finishing golden.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
