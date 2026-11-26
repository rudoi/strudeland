# 0127 loss check

The five stages cover the proposal in full. Nothing is dropped and nothing is out of scope.

| Proposal item | Stage |
|---|---|
| A/B guard at 416caa6: 181 refs plus 95 extra jobs (x01-x45, y01-y50), WAV and error and signature records, the `controls --json` dump, the live and stems digest; recorded twice; cached | 001 (before any engine edit) |
| `StageSpec` in `NodeTable`; `Voice::build` plans first and walks; one `build_stage` with today's throw order; `build_entry`, the debug assert and `push_stage`/`push_shaper` deleted; `StageKind::of` test-only; the kind check kept in `the_plan_lists_every_source_and_stage` (7 cases); `the_walk_throws_at_the_first_planned_site` | 002 |
| `FmStep`/`FmSide` planned; `apply_fm` walks them, both target-side `idl_float`s kept | 003 |
| `NodeName` with `as_str()`, string lookups kept; TARGETS cross-check (except `room`) | 004 |
| Structural tests moved onto the plan; voice-level tests on rendered output; the kept accessors; README and AGENTS.md wording | 005 |
| Zero change in sound bits, error texts and throw order, the target-log order, the dump, signatures, modulator targets, live timing | every stage (A/B check: 0 differences) |
