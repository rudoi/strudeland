# Real-memo evaluation for melody transcription

## Goal

Measure melody transcription against what a singer intended on real recordings. Use
the differences to guide improvements and keep the synthetic accuracy and runtime
budgets as regression checks.

## Scope

Start with an isolated copy of a user-selected memo and its recording sidecar. Keep
the original audio, automatic output, and the user's corrected melody as separate
local files. The first comparison may cover a phrase if that is what the user
can annotate. Additional recordings can test whether a change generalizes.
Publication of recordings and derived annotations requires separate permission.

## Architecture

Use the existing CLI to generate the baseline. Its transcription command resolves
the recording and timing metadata in `crates/strudeland-cli/src/run.rs`, and
`crates/strudeland-transcribe` produces pitch frames, detected onsets, and quantized
notes. Audition the result in Strudeland using a plain pitched voice. Use the
existing one-to-one onset-and-pitch scorer in
`crates/strudeland-transcribe/src/eval/score.rs` for the first accuracy report.

## Technical design

1. Record the exact command, CLI revision where available, and default JSON report
   with pitch frames and onsets. Verify that isolation preserves the report.
2. Align the corrected Strudel melody to the memo's bar numbering. Establish if
   the first group is a pickup or a full bar of silence. Score only the agreed
   portion and document what remains unannotated.
3. Report onset-and-pitch precision, recall, and F1 at 50 ms and 100 ms.
   Use the corrected grid as the musical reference. Score raw and snapped onsets
   separately. Report a fitted alignment alongside the original alignment and
   count any re-attacked ties as separate playback events.
4. Investigate two failure modes exposed by the first annotation. The circular
   phase estimator in `suggest.rs` cannot see whole-step offsets. Any automatic
   correction needs an independent timing anchor. In `segment.rs`, trace how
   runs with little or no voicing become notes, and check whether filtering them
   would preserve soft attacks and genuine pickups.
5. Reuse the synthetic
   suite and use additional annotated memos or a held-out phrase to check whether
   observed gains carry beyond the first case.

## Dependencies

The initial comparison needs a user-confirmed reference melody and its timing.
The CLI, Strudeland, and the existing scorer cover the experiment. Changes to the
pitch tracker or metronome handling depend on what the comparison shows.

## Risks

Expressive singing can move away from the intended pitch or timing. Score the
agreed musical reference and inspect the raw timing separately. A result from
one recording does not establish general accuracy. Keep private takes and their
transcriptions in local storage.

## Open questions

- Where does the intended melody enter relative to the count-in?
- Is the reference a phrase or the entire take, and are any slides intended notes?
- Which additional recordings can be annotated for independent validation?

Recommended complexity is small for the initial annotation and scoring pass.
Reassess implementation complexity after identifying which stages need changes.

## Refined implementation

The first local reference is an eight-bar intended-performance transcription.
Keep it as the first private fixture and run identical default and user-aligned
settings before and after implementation. Additional independently annotated
takes are needed before tuning pitch thresholds or generalizing from this score.

- Reject a proposed note in `segment::note_of` when none of its frames has both
  a decoded pitch and the existing minimum voiced probability. Preserve attack
  times, low amplitudes and pieces with even one supported frame. Compare an
  isolated unsupported piece and a quiet, minimally supported pickup.
- Preserve sustained notes in `notation::render_notation`. Quantized reports
  retain the per-bar parts and `tiedToNext` field for existing consumers. For
  visible ties, join only the exact contiguous continuation and render the
  resulting weighted sequence over the same number of cycles. Notes and scale
  degrees use the same timing. Keep existing output for passages without ties.
- Keep the sidecar downbeat and current shift controls. A whole-grid-step offset
  fits the same grid, and the local recording did not yield an independent
  audible timing anchor. Evaluate the user-aligned offset explicitly. Keep
  semitone ambiguities as alternatives. A tuning sweep changed individual
  spellings without improving the overall match.

Medium complexity. The segmentation and notation changes can be tested
independently. Publication stops at human PR review.
