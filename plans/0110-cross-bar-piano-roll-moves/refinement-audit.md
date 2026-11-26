# Refinement evidence

The current weighted mini parser confirmed the two-bar source example in [the
proposal](proposal.md). The focused source, grid, geometry and event suites passed 59
tests. The new work must prove the writer and the gesture together.

- The existing written-use index tracks names through timing transforms and sequence
  containers. It provides navigation but cannot prove phase or clock. The move needs
  a narrower placement check. Section playback resets phase to zero and clips notes
  at the section end, so check every affected repetition inside each section.
- A generated Expression fallback can suppress a source onset and then fail to recreate
  it if the pattern returns different notes at the destination's CPS. This occurred
  in a probe that returned C4 at 4/4 CPS and G4 at 7/8 CPS. The final-song planner reads
  the origin with the written CPS, but an inverse slice spanning a meter change can
  have multiple source contexts. Define an explicit guard for these cases.
- The persisted event interval uses existing gate scaling. The roll shows the logical
  whole, while a mask or clip may expose only part of that whole. For example, a note
  spanning portions of 4/4 and 7/8 and moved into 6/8 can preserve its whole in
  quarter-note units while its half-clip sounds for a different number of quarters.
- CodeMirror already applies disjoint patches in one isolated transaction and honors
  an explicit reveal patch. On undo, the dock can lose an explicitly selected meter
  anchor when the expression length changes. Validate the non-default anchor with
  a length-changing move in both history directions.
- The existing browser smoke harness can use the real headless JavaScript inspector
  with project IPC supplied by a temporary fixture. Native desktop validation uses
  the signed app launch and the CLI to verify the active buffer and evaluated events.
