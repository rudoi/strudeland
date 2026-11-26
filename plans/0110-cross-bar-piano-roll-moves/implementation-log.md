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

## Testing and staging

- Rebased against current `origin/main` at `77e210f`. The final frontend build,
  Svelte and TypeScript checks, and Prettier passed. `HOST_BUILD_CHECK=1 bun test ./tests`
  passed 3,718 tests across 131 files. The build reported its large-chunk warning.
- Signed macOS smoke opened a synthetic project and dragged E4 from `[7/8, 1)` in
  4/4 to `[1, 8/7)` in 7/8. A Left arrow reversed it. Each change undid in one step.
  The roll kept focus at the destination after the moves. The previous native project
  and active song were restored.
- The final Chromium smoke checked both current-page boundaries, shared-definition
  history and meter anchors, divergent repetitions, scoped save and reopen, equal-CPS
  absolute moves, and a masked note whose parts preview together. It used the real
  JavaScript inspector with an in-memory project transport.
- The first parity invocation found an older host binary with the previous request
  format. The branch host was rebuilt in the configured Cargo target. Chromium and
  the rebuilt host matched clip, tagged-loop, intro-loop, and folded-loop plans for
  both the standard mixed-meter fixture and a saved scoped move. In the saved fixture
  each E4 onset retained its 0.25-second duration at 120 BPM on both clip visits.
- The simplify pass covered the full task diff against `77e210f`. It moved drag
  arithmetic to meter runs, shared the preview code, checked each placement once,
  and verified saved event origins in their original phase.
- An independent high-thoroughness review covered the code through `b0f0450`. Four
  verified fixes landed at `8a007db` and were re-reviewed. They cover page boundaries,
  same-CPS meter changes, grouped masks during drag, and preview clipping for
  carry-in and carry-out notes. Keitora #148 tracks the remaining transient ambiguity
  when a true mask edge coincides with the viewport. The saved edit is exact.
- Updated the README and AGENTS.md and checked the local references in the plan and
  repository guide. Follow-ups #147 and #148 depend on this task.

## Final review

All six GitHub CI jobs passed on the reviewed head `51bdfd9`. The user approved
finalization and merge of [PR #33](https://github.com/rudoi/strudeland/pull/33).
The finalization commit removes the numbered execution files named in the entry point.
