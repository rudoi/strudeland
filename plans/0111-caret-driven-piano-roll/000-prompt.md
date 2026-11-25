# Caret-driven piano roll implementation

Branch `llm/andrew/strudeland/0111-caret-driven-piano-roll`, Keitora #111. Work in the current
user-created worktree. [Proposal](proposal.md), [refinement evidence](refinement-audit.md), and
[requirements cross-check](loss-check.md) are retained. The [editor](editor-experience.md),
[authored grid](authored-grids.md), and [event edit](event-edits.md) contracts are binding.

1. [Implementation](001-build.md) builds the source, runtime, and editor paths. Run focused checks
   with each change and keep separate commits for their code boundaries where possible.
2. [Validation](002-validate.md) exercises the assembled paths at the normal entry points.
3. Workflow staging performs simplification, high-thoroughness review, and the docs gate before
   publication. Stop at entry to review_gate. Finalization and merge require fresh approval.

Keep this file and the design and evidence documents at finalization. Remove `001-build.md` and
`002-validate.md` at finalization. Create `implementation-log.md` to retain actual validation,
which will be verified against the published head.
