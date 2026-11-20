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

- Start loop-only exports from the marked body's first downbeat with fresh loop effect state. For intro exports, render the intro through the first loop pass with contiguous effect history. Preserve sample-exact timing through both passes.
- Fold the isolated body's bounded post-roll into its repeat, using every period of decay covered by the render.
- Measure both joins independently. Move the final part of the opening toward the same effect state and phase as the end of the repeat. The intro decays through the first pass and never recurs in the loop.
- Use exact decoded PCM for WAV acceptance. Exercise a delayed first echo, wet reverb shorter and longer than the body, a wet intro crossing into the first pass, a dry loop, and a section with a distinct outro. Verify file roles, ordering, frame counts, markers, repeat behavior and shared gain. Verify decoded loop metadata for lossy formats and keep the existing MP3 gap warning.

## Dependencies and risks

The planner is shared by the browser and the headless host. Planner edits require the generated host bundle, engine reference provenance and associated parity checks. The CLI help, embedded skill and export documentation must describe the same file shapes. No new encoder or runtime dependency is anticipated.

The first transition and steady-state wrap can have different effect histories on short loops with long tails. A single fixed repeat cannot reproduce every intermediate traversal of a changing effect state. A player must honor nonzero loop points for the proposed single-file exports. A consumer that already loops the entire folded file would otherwise replay the opening.

## Resolved packaging questions

- `folded-loop` and `tagged-loop` keep a single file with the repeat marked.
- The split opening keeps the existing `-intro` suffix.

Recommended workflow complexity is medium. Keep a single proposal, skip fracture, and retain singletonize and a light loss check because the host, renderer, encoders and CLI share the contract.

## Refinement decisions

Retain the existing preset names, file counts for the two single-file presets, and the split suffixes and roles. The split preset always emits two files. The legacy `-intro` name now means the one-time opening, even when the song begins on its loop. Single-file consumers start at frame zero and repeat the marked range. Their marker starts after the first traversal, including any intro.

Reuse bounded modular post-roll folding for the periodic reference. When the opening starts on the loop, one engine render supplies both versions. When it has an intro, plan the continuous opening and a separate isolated body with post-roll in one evaluation. Use identical authored bar coordinates for both copies. The length of a body can differ by one frame when measured against a shifted absolute bar, so do not plan the second copy at shifted bar coordinates. Preserve the original body event cycles and seeds.

The recurring file contains only the isolated loop. In the one-time opening, use a smoothstep blend over the final 10 ms at most, capped at half a body, between its final frames and the corresponding final frames of the repeat. An opening already matching the repeat at that phase stays identical. This keeps the opening downbeat and the intro handoff intact, keeps intro-only sound out of the repeat, and makes the first wrap use the same boundary waveform as subsequent wraps. At very long decays it transitions into the periodic state at the end of the first pass. Modular folding uses the same post-roll approximation as the existing preset. A simulation of nonlinear effects over many successive traversals is outside this revision.

Verify two paths through the planner, both file joins in WAV, exact decoded lengths at a fractional-frame tempo, unchanged seed identity, and upper bounds on assembly. Preserve one-shot and legacy plan responses.
