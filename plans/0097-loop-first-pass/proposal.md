# Clean first pass for loop exports

## Goal

Play the loop once with the effect state that follows from the music heard so far. Then enter a repeat region that carries the loop's own releases, reverb, and delay across its boundary.

## Scope

| Preset | Proposed playback |
| --- | --- |
| `intro-loop` | Play an opening asset containing the intro and first loop pass once. Switch to a repeat asset and loop it from frame zero. For a song with no intro, the opening asset contains only the first pass. |
| `folded-loop` | Start a single file at frame zero with the first loop pass. Loop from the start of the repeat pass through EOF. |
| `tagged-loop` | Start a single file at frame zero with the intro and first loop pass. Loop from the start of the repeat pass through EOF. |

Each pass occupies one body-length in decoded frames. The assets exclude authored outro bars. One-shots, regular renders, and live playback are outside this feature.

## Architecture

The [previous loop export contract](../0015-loop-export-modes/design.md) isolates the intro from the loop in the split preset. The new opening is continuous. Existing intro effects can cross into the first pass and can decay naturally. Recurring audio is derived from the loop body so an intro-only sound cannot recur every time the repeat asset wraps.

The `exportWindow` and `planRender` functions in `src/lib/render/job-plan.ts` currently resolve one musical pass. `planRender` isolates the split loop, estimates decay, and omits post-roll for the tagged preset. The new plan needs to provide PCM for a contiguous opening and for the body with loop-only effect history. Reuse a single song evaluation and keep the repeat's musical content tied to the same selected body, including deterministic events and seeds. Reuse the existing timing, buffer guards and chunk machinery.

`render_export` in `crates/strudeland-render/src/export.rs` currently applies `fold_tail` in place before it partitions the PCM. Assemble the opening and repeat before publishing any files. Keep one shared static gain and include both the opening-to-repeat join and the repeat wrap in true-peak handling. The existing encoder loop metadata and CLI JSON loop points describe the repeat region in decoded frames. Update output naming, role reporting and playback examples together if split-file naming changes.

## Technical design

- Start mono or stereo loop-only exports from the marked body's first downbeat with fresh loop effect state. For intro exports, render the intro through the first loop pass with contiguous effect history. Preserve sample-exact timing through both passes.
- Evaluate a repeat candidate with real prior body history. Compare against the existing periodized post-roll as a reference, especially where delay feedback, heavy effects, or a decay longer than the body make the second traversal differ from a steady-state traversal.
- Measure both joins independently. If a long decay keeps the opening and periodic state apart, use a bounded transition in the repeat region only if fixtures demonstrate that both joins improve and the intro never repeats. Specify the seam policy in refinement before implementation.
- Use exact decoded PCM for WAV acceptance. Exercise a delayed first echo, wet reverb shorter and longer than the body, a wet intro crossing into the first pass, a dry loop, and a section with a distinct outro. Verify file roles, ordering, frame counts, markers, repeat behavior and shared gain. Verify decoded loop metadata for lossy formats and keep the existing MP3 gap warning.

## Dependencies and risks

The planner is shared by the browser and the headless host. Planner edits require the generated host bundle, engine reference provenance and associated parity checks. The CLI help, embedded skill and export documentation must describe the same file shapes. No new encoder or runtime dependency is anticipated.

The first transition and steady-state wrap can have different effect histories on short loops with long tails. A single fixed repeat cannot reproduce every intermediate traversal of a changing effect state. A player must honor nonzero loop points for the proposed single-file exports. A consumer that already loops the entire folded file would otherwise replay the opening.

## Open questions

- Confirm the proposed packaging, especially whether `folded-loop` should keep one file with a nonzero loop start or produce two explicitly managed assets like `intro-loop`.
- Confirm that `tagged-loop` should use the same two-pass behavior. Its current one-pass contract could instead be retained for compatibility.
- Decide whether the split opening should retain the existing `-intro` suffix for compatibility or be renamed `-opening` to describe its new contents.

Recommended workflow complexity is medium. Keep a single proposal, skip fracture, and retain singletonize and a light loss check because the host, renderer, encoders and CLI share the contract.
