# Cross-bar piano roll moves

## Goal

Move a note into another written bar while preserving its duration in quarter-note units.
Write the move into the original notation when it can represent the result exactly. The
preview, saved source, and evaluated song should agree on the destination onset and end.

## Scope

- Support pointer moves and keyboard nudges between bars on the current roll page. Snap the
  onset to the destination bar's subdivision or authored-slot grid. Retain free snap.
- Prove direct source edits for eligible weighted mini-notation bars and recognized note
  grids. Use the existing scoped event-edit path where it can represent other moves.
- Preserve existing pitch, resize, insertion, and deletion behavior. Keep page navigation
  separate from the gesture. Add no notation syntax or song metadata.

## Architecture

The [roll dock](../../src/components/PianoRoll.svelte) chooses the written meter anchor and
edit scope. The [grid component](../../src/components/PianoRollGrid.svelte) snaps pointer
and keyboard input. Give both inputs the same move calculation based on the meter behind
the ruler. Use written bars directly in In song and map expression cycles through the
section anchor in Expression. Use the ruler's reference meter for unanchored expressions.
Calculate with the note's whole interval, including meter coverage outside the viewport.

Pass resolved start and end together to the source planner, scoped event planner, and
[feedback projection](../../src/lib/piano-roll/feedback.ts). Today those paths derive the end
from the original cycle duration. Use the [project session](../../src/lib/project/session.ts)
for the revision check, destination reveal, and isolated undo transaction.

## Technical design

### Musical time and snapping

Split the whole source span at written bar boundaries. Multiply each segment by its bar's
quarter-note length, calculated as four times the numerator divided by the denominator.
Spend that exact duration from the snapped destination, advancing through bar boundaries as
needed. Use Strudel fractions for musical positions and duration. Pixels choose the snap
point. For a carry-in note, preserve the musical offset between the grabbed visible edge
and the onset. A boundary onset uses the following bar's meter and a note may end at the
same boundary. Keep existing same-bar behavior for notes that remain wholly inside one bar.

Tie a gesture to its source revision, song, and meter anchor. A change to any of these
invalidates the gesture. Verify that the destination stays inside the editable scope.

### Direct notation and scoped event edits

- Extend the [mini phrase codec](../../src/lib/piano-roll/phrase.ts) to serialize the vacated
  and destination bars together when each interval fits within its respective bar. Reuse
  the bounded rational-weight encoder. Preserve the moved pitch spelling, sibling bars,
  phrase period, and enclosing JavaScript. Reparse the combined candidate and compare
  the exact pitched event multiset. Mark the destination patch for reveal in both directions.
- Use both resolved edges in the [authored grid codec](../../src/lib/piano-roll/authored-grid.ts).
  Require both edges to land on integral slots within the finite grid. Permit holds across
  rows where the consumer supports them. Preserve the suffix bytes and other runs. Verify
  and show any bend changes caused by reordered pitched runs.
- Touching endpoints are available in the monophonic codecs. Try a [scoped event edit](../../src/lib/piano-roll/event-source.ts)
  for an overlap, a cross-bar sustain outside the mini grammar, or an off-slot edge. Use
  the existing event entry's `until` field for the resolved end and preserve event identity
  and relative gate semantics. Check exact behavior through repeats and query partitions.
  The duration invariant applies to the whole interval shown by the piano roll. Clip,
  explicit sound duration, and partial masks retain the existing event-edit scaling rules.
  When scope, source meter context, or entry limits prevent an exact edit, keep the source
  intact and explain the constraint.

### Shared definitions

Keep occurrence edits local through the existing scope resolver. The selected anchor
determines the proposed interval. Verify each placement of the edited expression, including
repetitions within a section. A definition edit can proceed when the same logical interval
preserves the note's musical duration and fits at every written use. A definition-wide event
wrapper needs the same check. When placements diverge or cannot be proved, show the reason
and point to the specific occurrence in In song. Keep the existing shared-definition controls.

## Dependencies and evidence

- Task 109 established the [source contract](../0109-meter-aware-piano-roll/source.md) and
  [exact two-bar example](../0109-meter-aware-piano-roll/refinement-audit.md#focused-probes).
  Task 111 added [authored grids](../0111-caret-driven-piano-roll/authored-grids.md) and
  [scoped event edits](../0111-caret-driven-piano-roll/event-edits.md). The grid codec already
  patches multiple rows using a fixed count of logical slots.
- The current Strudel projection confirmed that `<[c4 e4 ~@2] [~@5 g4 ~]>` can become
  `<[c4 ~@3] [~@3 e4@2 g4 ~]>`. E4 moves from `[1/4, 1/2)` in 4/4 to `[10/7, 12/7)`
  in 7/8, preserving one quarter note. C4 stays at `[0, 1/4)` and G4 at `[12/7, 13/7)`.
  The current mini planner declines that move. The current grid move and event preview
  preserve cycle length, so both need the same resolved musical interval.

## Risks and verification

- A candidate can shift a neighbor, change the phrase period, or change an adjacent glide.
  Compare exact events against vendored Strudel queries and check byte preservation outside
  the patches. Cover both directions, rest-only and bare bars, and bounded weight failures.
- Exercise 4/4, 7/8, and 6/8 boundaries, exact fits, cross-bar sustains, overlap, off-slot
  edges, carry-in notes, and shared definitions with equivalent and divergent anchors.
  Verify saved edits with whole and partitioned queries and loop and clip planning. Run
  mixed-meter runtime and render parity checks if the runtime path changes.
- In the desktop app, check pointer and keyboard parity, immediate preview, destination
  reveal, retained focus and meter anchor, and undo/redo in one step. Save, reopen, and
  re-evaluate a direct edit and a scoped fallback.

## Decisions from refinement

- Keep duration preservation mandatory for a shared move. In song can edit a particular
  written occurrence with its own inspected identity.
- Move between bars on the current roll page. Use existing navigation to reach other pages.
- Verify semantic placements using stricter evidence than the section names used by the ruler.
  The [implementation contract](contract.md) defines accepted placement and fallback cases.

Recommended complexity is medium. The gesture, source codecs, scoped fallback, and tests
share one interval contract. Keep singletonization and a light loss check before sequencing.
