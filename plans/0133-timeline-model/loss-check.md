# 0133 loss check

The one stage covers the plan in full. The proposal's Out list (action wrappers, the
stop-looping `disabled`, `commitProblem`, every other file) stays out by design.

| Plan item | Stage |
|---|---|
| `TimelineViewInput`, `TimelineView` and `timelineView` in `timeline.ts`, the moved expressions with their operand order, the module doc line, the three carried comments | 001 |
| The ending and stop-looping help texts move byte-identical; the literal-count script proves every moved and kept string | 001 |
| `Timeline.svelte` reads one destructured `$derived`; the two `help=` attributes read the strings; the whole file equals `b0229a8` with exactly those edits (markup and whole-file checks) | 001 |
| Tests: another song plays, evaluated structure differs from the header, armed while stopped (stale position), duplicate and missing names, no located position, `ending` from `playable`, `pct` with and without sections, the audition names, the meta-error and no-structure armed gates | 001 |
| The 5-row ending help table, its precedence row (an unreachable state, kept as a test of the pure function's order) and the 3-row stop-looping table | 001 |
| Existing tests and every other file frozen; svelte-check counts equal the base log `0133-001-base-check.log` | 001 |
| The AGENTS.md `timeline.ts` entry | 001 |
