# Cross-bar move contract

## Resolved interval

A move carries its exact start and end as reduced fraction strings in the current roll
coordinate system. Pointer and keyboard snap a visible edge to a grid position. Preserve
the whole note's quarter-note length and the musical distance from its onset to that
edge. Use the same meter lookup for the move and the ruler. Reject a move whose source or
destination falls outside the applicable written range. A destination end at the last
boundary is valid. Retain existing resize behavior.

## Placement proof

For a cross-bar write in Expression, trace from the actual source expression receiving a
rewrite or wrapper. Follow immutable aliases, grouping, and proven time-transparent
operations to place it in a section of the song or on an independent base clock. Follow
the enclosing song to its output as well. The general reference index is used for
navigation and includes transformations that change phase. Traversal through unknown
calls or timing-changing operations cannot establish the invariant. A known unused
definition may use the base clock.

For each reached section, phase starts at local cycle zero. For a phrase or wrapper
with a proved period, compare the original and destination spans at every affected
period offset in that section. A live or exported loop replays those same placements.
A nonperiodic event uses the single inspected phase. If a span intersects the section,
both spans must fit completely and have the same exact length in quarters measured
at that placement. Use this check even for one section or an inline phrase. When placement
is uncertain or incompatible, direct the user to In song for a particular occurrence.

## Authored writes

Plan both mini bar changes in original editor coordinates. Reparse the combined literal,
query the projected events, and compare entire pitched event multisets and phrase length.
Reparse the entire script and compare its shape with that literal masked. Recognized grid
writes patch cells and recheck whole runs, preserving suffixes and showing dependent bend
changes. The existing transaction commits the full patch set. Explicitly mark the
destination patch for reveal, including leftward moves.

## Event writes

Reuse the source-backed edit format and its validation limits. Store the exact end in
the original entry when the cycle duration changes or the entry already has an end.
When moving a later repeat of an existing periodic edit, normalize both new edges to the
entry's canonical phase. Keep the original match identity and any earlier retune. Reject a
phase offset that is not an exact multiple of the saved period.

The existing event edit scales mask fragments affinely in logical cycles and applies its
existing clip and duration rules. The piano roll's duration guarantee applies to the whole
note. Project the visible fragments of an edited note together when their identity is
known and leave unrelated unisons alone. During refresh, the inspected fragments are
authoritative.

In Expression, the source of a generated note can depend on the query CPS. Keep
mixed-meter fallback to source material whose time and values are proved independent of
query CPS. A generated fallback can use the same query context when the source and
destination all use the same CPS. In song, a written origin probe can use the source
meter when the source span has one CPS. If a source spans a change in CPS, or an
absolute placement can read another CPS when its transport route changes, require proof
before committing the generated edit. Leave unsafe input untouched with a specific
explanation. These checks leave the event runtime and song format intact.

## Interaction and history

The preview, optimistic result, focus match, and persisted move all consume the same
resolved edges. Invalidate on a stale source, view, song or anchor. Keep the selected
meter anchor through a direct change and its undo/redo. Test a size-changing patch and
an explicit non-default anchor. Save and reopen a direct edit and a scoped fallback.

Validate the quarter-note example from the proposal in both directions, a fixed grid with
holds, a compatible and divergent repeated placement, a bounded overlap fallback, and
the source-CPS failure described in the refinement audit.
