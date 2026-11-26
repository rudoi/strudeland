# Exact move planning

Goal. Resolve the whole interval exactly, and prove each writer applies it safely.

Prerequisites. The proposal, contract and reconciliation are committed.

Implementation. Implement meter arithmetic and the move payload. Extend the source codecs
and source placement analysis. Verify the saved event interval and its phase. These writer
changes may proceed in parallel in their own files after agreeing on the payload.

Validation. Run the focused roll geometry, source, grid, event, and feedback suites. Include
the pinned 4/4 to 7/8 example, compound meters, cross-row holds, overlapping voices,
fragmented events, periodic entries, and rejected source forms.

Commit. `feat(editor): plan exact cross-bar note moves`
