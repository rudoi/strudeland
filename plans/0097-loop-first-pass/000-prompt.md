# Implementation sequence

Branch `llm/andrew/strudeland/0097-loop-first-pass`. See [the refined proposal](proposal.md) and [contract check](loss-check.md).

1. Update the shared planner and host response for a continuous opening plus optional separate, isolated repeat plan. Preserve authored bar coordinates and legacy responses. Cover plan/export limits, lookback and event reporting with planner and host tests. Regenerate the host bundle and engine references. Commit as `feat(render): plan opening and repeat loop audio`.
2. Render and validate all jobs, assemble the clean opening and folded repeat, condition the handoff and finish both with a shared gain. Update CLI output handling and docs, and check WAV playback and decoded metadata in the render and CLI suites. Commit as `feat(render): export a clean first loop pass`.
3. Validate on fetched main with canonical Rust and frontend gates, host parity and engine reference/vector checks, plus a CLI export smoke test. Run simplify and high code review, update records and publish the PR. Pause at human review. Tests and gate results belong in the implementation log. There are no temporary stage files to remove.
