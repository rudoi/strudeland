# 004: the loop splice

## Goal

Move the loop arrangement out of `loop_export.rs` into a pure `splice.rs`. It covers the
opening copy, the tail fold, the opening join, the wrap peak and the file cuts. Its rules get
direct tests on synthetic PCM. `finish_loop` consumes `FileCut`s, which replace `Part`. There is
no behaviour change.

## Prerequisites

- **Stage 003 is committed,** and the characterization passes.
- **Read the proposal's** "Loop splice (stage 004)" and the glossary entries **File cut** and
  **Loop splice**.

## Implementation

1. **Create `src/splice.rs`** (`mod splice;` in `lib.rs`), with the module doc: "The loop
   splice: the continuous opening-plus-repeat PCM a loop preset is cut from, and its file cuts."
   - **`FileCut<'a>`** and **`Splice`**, as in the proposal. A `FileCut` from the splice owns its
     channels (`Cow::Owned`). `FileCut` is defined here for good: stage 005's `finish.rs`
     imports it from `splice` rather than moving it.
   - **`splice(...)`** is `loop_export.rs:75-147` moved verbatim. Its inputs:
     - `channels`, which was `audio.channels`;
     - `music_frames`, which was `audio.music_frames`;
     - `window`;
     - `sample_rate`;
     - the destructured `LoopAudio`;
     - `preset`.

     It returns `Splice { cuts, program, peak_db: program.true_peak_db.max(wrap) }`. Keep the
     `check_input(&opening)?` before `measure`. Keep `drop(repeat)` in the single-file arm.
     `ExportPreset::OneShot` still returns "unexpected one-shot preset".
   - **Move `copy_opening`, `join_opening`, `fold_tail` and `wrap_peak`** with their doc
     comments, and `render_error` (or import it).
2. **`src/loop_export.rs`: `finish_export`'s loop branch.**
   - Keep `loop_audio.ok_or_else(|| render_error("the plan has no loop start"))?` here.
   - Move the window, frame and length checks into `splice`, and start `measure_start` just
     before the call. The level time then also covers those cheap checks. Timings are
     wall-clock and not pinned.
   - Call `splice(audio.channels, audio.music_frames, audio.summary.window,
     audio.summary.sample_rate, loop_audio, preset)?`, then set `summary.timings.level_ms =
     ms_since(measure_start)`.
   - Call `finish_loop(spliced.cuts, summary, &opts.output, spliced.program, spliced.peak_db,
     progress)`.
   - **`finish_loop`** takes `Vec<FileCut<'_>>`; rename `part` to `cut` and read `&cut.channels`.
     Its logic is unchanged.
   - Delete `Part`.
3. **Keep the error order exactly** (`loop_export.rs:74-107` at the plan commit):
   1. "the plan has no loop start" (in `finish_export`);
   2. "loop start is outside the planned window";
   3. "loop start is too large" (the start frame's conversion);
   4. "loop end is too large" (the end frame's conversion);
   5. "loop has no frames";
   6. "the combined loop is too large";
   7. then `copy_opening`'s error ("opening extends past the rendered PCM", "cannot allocate
      the loop opening") in the bar-0 arm, or "the plan has inconsistent loop history";
   8. then the fold, the join and `check_input`.

## Tests

**`splice.rs` `#[cfg(test)]`:**

- **Moved:** `folds_full_and_partial_repeats_into_a_body`,
  `the_opening_blend_matches_the_repeat_without_a_step_in_the_fade` and
  `measures_a_wrap_even_for_a_loop_shorter_than_the_filter`. Delete them from `loop_export.rs`.
- **Fixture helper:** `fn fixture(start_bar, isolated: bool, tail_frames)` builds a 4-bar window
  at a small synthetic rate (e.g. `sample_rate` 1000, cps 1.0) with distinct constant blocks
  per bar and a distinct tail.
- **`intro_loop_cuts_an_intro_and_a_loop`:**
  - Roles are `[Intro, Loop]`.
  - The intro has `music_frames` frames, bars `0..4` and no range. Its last 10 ms blend into the
    loop's last frames.
  - The loop has bars `2..4`, range `0..body` and `body` frames, with the tail folded in.
- **`tagged_and_folded_cut_one_file_with_the_repeat_marked`:**
  - Roles come from `roles()`.
  - The range is `music..music+body`, and the channels are the joined opening plus the folded
    repeat.
  - Tagged and folded give identical channels.
- **`a_loop_at_bar_zero_folds_a_long_tail_more_than_once`:** a tail of 2.5 bodies. The body is
  the sum of the three cycles. The opening is a copy of the first `music` frames joined to it.
- **`splice_errors_name_the_bad_plan`:** exact `to_string()` for each of:
  - start bar outside the window ("render failed: loop start is outside the planned window");
  - start frame equal to `music_frames` ("… loop has no frames");
  - an isolated body at start 0, and none at start > 0 ("… the plan has inconsistent loop
    history");
  - a body shorter than the loop ("… loop extends past the rendered PCM");
  - a ragged opening (`check_input`'s text).
- **`the_peak_covers_the_wrap`:** a body with a step at the wrap gives `peak_db >
  program.true_peak_db`. The program's handoff mirrors the wrap through the 10 ms blend, so the
  wrap is strictly higher only by the blend's residue: give the opening a last 10 ms that
  opposes the repeat's tail. A smooth one gives `peak_db == program.true_peak_db`.

## Validation

- **Targeted:** `cargo test -p strudeland-render splice::`.
- **The stage gates,** logged once. `characterization` passes, and the golden is untouched.
- **`loop_export.rs`** now holds the public types, `render_export`, `finish_export`,
  `trim_silent_post_roll` and `finish_loop`.

## Commit message

```
refactor(render): move the loop splice into its own module

The opening copy, tail fold, opening join, wrap peak and the cutting of
a loop preset's files move from loop_export.rs into a pure splice
module that returns file cuts and the peak the shared gain must
respect. Its rules are tested directly on synthetic PCM.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
