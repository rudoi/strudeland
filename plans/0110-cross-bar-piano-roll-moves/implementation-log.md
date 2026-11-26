# Cross-bar piano roll implementation log

## Planning and integration

The user authorized automatic refinement and advancement through entry to the Review
Gate. The [audit](refinement-audit.md) records the placement, origin-CPS, and history
findings. The task uses the existing checkout and configured Cargo target directory.

Exact interval arithmetic and source planners landed in `7e87ed8`. The pinned
4/4 → 7/8 example passes in both directions. Authored grid tests verify fixed slots, cross-row
holds, collisions, suffix preservation, and neighboring glide changes. The saved-event
tests cover resolved end, relative gate and mask projection, overlapping voices, query
partitions, and edits to a later repetition. The focused run passed 129 tests across six
files. Svelte and TypeScript reported zero errors and warnings.

Pointer and keyboard share the same calculation and meter lookup. The dock checks scope
before a cross-bar write and preserves the chosen meter anchor when undo restores earlier
source offsets. A Chromium smoke using the real JavaScript host inspector and an in-memory
project transport passed direct pointer and keyboard writes, exact neighboring events,
focus, one-step undo/redo, and an In song move through save, reopen, and evaluation.

Keitora #147 tracks source query context for remaining generated mixed-meter moves. It is
blocked by this task. The guard reports when a scoped move cannot preserve the source
query context.
