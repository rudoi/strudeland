# Caret-driven piano roll and editable song grids

## Goal

Place the caret inside pitched material and start editing from the piano roll while stopped. Work with a note pattern at its use or shared definition, and move through a multi-bar grid without copying it into a different notation. When editing is unavailable, explain the reason and offer a source action that fits the pattern.

## Scope

- Infer Expression targets from a single caret. Keep an explicit selection as the override. Keep the persistent toggle, fitted pitch range, auditioning, shared-use display, and the In song view from [the original roll](../0109-meter-aware-piano-roll/proposal.md).
- Use Luoyuan’s eight-bar flute grid as the acceptance case for authored arrays. Open on the row under the caret, provide access to all eight bars, and show continued notes in any row they overlap. Support pitch and row-local rhythm edits to the extent each gesture can be written safely. Prove at least one representative edit through the actual consumer.
- Give existing mini notation and immutable aliases a fast source path. Inspect transformations and generated expressions through the bounded headless host. Explain the specific constraint or missing context when source editing is unavailable.
- Provide a reusable way to author editable grids and prototype an explicit **Make editable** preview for compatible patterns. Preserve source until confirmation. Keep arbitrary generated-pattern flattening, velocity and pitch-envelope editing, and general cross-bar gestures outside this pass. Cross-bar gestures are tracked in Keitora #110.

## Evidence from Luoyuan

The matching script was found in the local render fixture `target/underflow-profile/input/songs/original.js`. It declares a 120 BPM, 4/4 song with an eight-bar intro. The complete script stays in the local project. A synthetic regression fixture can exercise the same behavior with different pitches.

| Source stage | Observed behavior |
| --- | --- |
| `fluteGrid → fluteSteps` | Eight quoted rows, each trimmed, split on whitespace and checked for exactly 16 slots. The rows are flattened before interpreting holds. |
| `fluteSteps → fluteEvents` | Pitch starts a note. A following run of `_` extends that event across row boundaries. `~` starts silence. A leading `_` without a sounding onset becomes silence. |
| Pitch suffix | A decimal suffix becomes `pattack` in seconds. When a previous pitch exists, the consumer also computes `penv` from the pitch interval. The previous pitch persists through rests. The defaults are 0.09 seconds and a bend of 0.55 where the relevant glide inputs are absent. |
| `fluteEvents → fluteCall → intro` | `stepcat(...fluteEvents).slow(fluteGrid.length)` makes a slot exactly 1/16 of a cycle. The intro layers the authored line through a wrapper that adds a pitched sine voice and a noise voice. |

The existing host query over the intro returned ten orbit-12 sine onsets. The opening A5 begins at cycle zero and lasts 27 slots, ending at the G5 onset in bar 2. At 120 BPM the output duration is 3.375 seconds. The G5 with suffix `:2` lasts five slots, or 0.625 seconds, and the host reported its bend as −2 semitones. The suffix interpretation comes from the source helper. The CLI query summary does not expose `pattack`. Verify that control with direct queried values or engine jobs during implementation.

A pitch change can therefore intentionally change the current or next gliding note’s bend even when the other note’s pitch, duration, and source text stay fixed. The test oracle must verify that coupled effect. A hold at the start of bar 8 may belong to the D5 onset in bar 7.

## Architecture

Keep source resolution, musical projection, and application of code edits as separate steps.

1. **Editor context.** Extend [`pianoRollSnapshot`](../../src/lib/project/session.ts) to distinguish a caret, an explicit range, and unsupported multiple ranges. [`setup.ts`](../../src/lib/editor/setup.ts) already reports caret and document updates. Reuse it. Resolve the caret against editor syntax and verify ownership with the source resolver’s Acorn binding analysis. Cache source analysis by the active snapshot so cursor motion does not reparse the document or issue a new host request for the same expression.
2. **Source target.** Extend [`resolveRollTarget`](../../src/lib/piano-roll/source.ts) to return a stable target identity and typed source capabilities. Resolve a pitch-bearing value to its smallest meaningful expression. Follow known immutable uses to their definitions with existing scope and affected-use analysis. For grids, retain grid identity, focused row, definition/use and exact source spans. Keep the occurrence that determines the meter separate from the authored grid identity.
3. **Projection and edit planning.** Retain the [mini-notation phrase codec](../../src/lib/piano-roll/phrase.ts). Give an identified grid consumer a local codec that decodes all rows into exact intervals and can plan local token edits. Both codecs return patches through the existing source validation and [`editPianoRoll`](../../src/lib/project/session.ts) transaction. The roll uses their declared capabilities to enable individual gestures.
4. **Fallback and authoring.** [`PianoRoll.svelte`](../../src/components/PianoRoll.svelte) uses authored notes when available. It keeps [bounded host inspection](../../src/lib/piano-roll/inspection.ts) for evaluated expressions and the song view. The dock maps reason codes to an appropriate action, such as inspecting the expression, opening the shared definition, navigating to an onset, or previewing a supported source conversion.

## Technical design

### Caret inference and dock state

Keep the existing strict explicit-selection resolver for complete expressions and pitch atom ranges. Caret candidates include pitched literals, known pitched constructors and safe receiver chains, immutable declarations and references, and rows owned by a verified grid. Use lexical ownership to exclude gain, pitch-envelope and other effect arguments, object keys, and comments. Allow intra-pattern whitespace and closing delimiters when the caret still belongs to an unambiguous value. Clear the target on an unrelated line. An incomplete edit may retain the last pane context for orientation, with its write actions disabled until the current source can be resolved.

Currently the dock keys its page, anchor, and shared-definition view to a selection revision that changes with caret movement. Key those by project, song, and resolved target or occurrence. Keep the exact document and selection/target checks for committing a gesture. Snapshots from another song or old source must be rejected, and a change of active target cancels a pending gesture. CodeMirror edit transactions should map a caret as a caret and keep explicit ranges selected where possible. A roll edit should undo in one step. Crossing into another grid row updates the focused row without discarding the song anchor.

### Grid identification and write-back

Prototype a deliberately narrow syntax-and-binding match for the observed Luoyuan pipeline, with a parity test before enabling write-back. Check the complete chain that defines timing, rests, and glides, including local shadowing of builtins, immutable aliases, mutations, and unknown consumers. If the proof becomes brittle, use an explicit previewable adoption of a built-in grid helper with equivalent behavior. Support both a direct path for confirmed legacy code and a documented helper only when their behavior can be verified against the same oracle. A quoted array that happens to contain pitch-like words stays view-only until its consumer is established.

Decode one slot as 1/16 of a cycle for the observed pipeline. Use exact fractions and the existing [meter geometry](../../src/lib/piano-roll/grid.ts) and [song structure](../../src/lib/strudel/structure.ts). Anchor written bar labels and meters to a known song occurrence, with the existing preview meter and occurrence chooser as fallbacks. Keep slot snapping independent of visual beat grouping. Clip long notes for display but retain their actual onset, end, and source origin. A row-leading continuation links to the onset row.

For a retune, replace only the pitch characters and keep the glide text. For rhythm gestures, serialize a fixed number of cells and check that the rebuilt grid yields the requested event plus expected glide dependencies. Retain quotes, comments, delimiter whitespace, sibling rows, and unaffected tokens. When an edit would disturb another onset, extend across a row boundary, or change a duration that cannot be represented on the source lattice, surface the specific restriction. Reject changed array structure, escaped or computed rows outside the proven format, and unexpected changes in consumer behavior. An audition previews pitch using the existing voice/fallback path. The scheduled glide remains the song consumer’s responsibility.

### Explainable inspection and conversion

Differentiate unresolved targets, recognized view-only expressions, missing inspection approval or runtime support, and edit limitations on otherwise writable notes. Offer fast local projection when syntax establishes pitches and time. Preserve the existing opt-in for inspecting unevaluated source and the host’s request bounds and stale-result handling. Runtime onsets alone do not establish a writable source origin.

Start the conversion prototype with a finite, deterministic pitched pattern for which the candidate matches exact queried events and relevant control values. Show the proposed text, affected uses, and any deliberate control changes before accepting. Recheck the document and target at confirmation and apply in one undoable transaction. Retain the current voice chain and section attachment where supported. Offer a documented editable construction when the conversion cannot establish parity. Keep generated output inspectable.

## Dependencies and validation

- Existing project session transaction and lifecycle rules, [editable phrase tests](../../tests/piano-roll-source.test.ts), [geometry tests](../../tests/piano-roll-grid.test.ts), and [host inspection tests](../../crates/strudeland-host/tests/inspection.rs). No dependency on Keitora #110.
- Review local song forms during refinement, including Luoyuan’s authored arrays, weighted alternate rows via `note(...)`, stacked chord rows, shared voices, and rhythm-only patterns. Record a capability matrix and the first unsafe boundary for each form.
- Exercise caret boundaries, effects and comments, nested and shadowed calls, incomplete source, alias definitions and uses, target stability, and explicit selection precedence. Verify caret-only editing while stopped and inspection deduplication.
- Verify eight-row navigation, sustained notes across boundaries, pending gesture cancellation, exact gesture round trips, mixed meter, suffix preservation, expected glide dependencies, unchanged neighboring events, and one-step undo. Use synthetic source in the repository and query the local Luoyuan intro for acceptance.
- If a built-in song helper ships, use the same implementation in playback and both rendering paths. Verify the render page and headless host agree, rebuild the committed host inputs and bundle when required, and compare pre/post conversion audio jobs. Run the workflow’s frontend gates and a desktop smoke covering the toggle, an actual row edit, auditioning, and undo.

## Risks and open questions

- The legacy parser sits inside general JavaScript. A superficial name or content match can expose edits for arrays with different timing. Refinement must establish the smallest trustworthy recognition test and determine when the app should offer conversion instead.
- A movement that reorders onsets can change glide relationships even if every cell edit is encodable. Validate expected control dependencies and decide whether the first implementation should restrict these gestures to preserving onset order.
- Proposed default is the row under the caret, with adjacent bar navigation and a whole-grid page view. Verify whether the desktop workflow benefits from keeping multiple adjacent rows visible by default, especially while editing a sustained note.
- Proposed authoring route is a built-in helper if it can unify legacy conversion and new grid authoring without changing the event schedule. Refine its API after testing the legacy matcher and the conversion preview.
- Large, cross-cutting work. Keep fracture, singletonize, loss check, and sequencing to verify the editor, source-editing, runtime parity and acceptance contracts before implementation.
