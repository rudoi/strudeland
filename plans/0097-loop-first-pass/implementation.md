# Implementation and validation

The planner evaluates and selects the song's loop export schedule once. It plans the opening continuously through the body. If there is an intro, it also plans the same selected body in isolation. When the opening already starts at the body, its original PCM and bounded post-roll supply both passes. The renderer folds the body post-roll, aligns the end of the one-time opening, and assigns loop markers to the repeat. The CLI retains the existing preset names and file roles.

The implementation was rebased onto `8ff2acb`, which added File menu project actions. The two render variants use the same authored frame rounding. The 137 BPM, 7/8 WAV test confirms the second copy has exactly the first body's frame count. Other cases verify wet intro carryover, delayed and reverberant loop audio, absence of intro audio in the repeat, shared peak handling, loop tags, and complete worker progress.

## Checks

- Frontend production build, Svelte and TypeScript checks, and Prettier passed. The final repository Bun suite passed all 3,197 tests, including a fresh host build comparison. The run excluded archived test trees under the existing Cargo target. An early scoped run overlapped an older Bun discovery run and tripped the vendor sync test that compares the global temporary directory. The isolated reruns passed.
- Workspace Rust tests passed on the rebased implementation with 2,113 passing, six ignored, and the same `dropout_short` exclusion as CI. The final workspace Clippy and formatting checks and the affected host, render, and CLI test suites passed.
- The host/page planner and batch planner parity runs each passed 34 of 34 corpus fixtures.
- Engine and render vectors and the final Chromium references were current. Regeneration updated provenance on 181 jobs, with identical payloads, and refreshed the two accepted measured-floor WAVs. An earlier reference check passed on retry after an isolated transient-attack check reproduced the committed job. The superdough goldens are unchanged.
- A real CLI smoke run produced all three WAV layouts and the expected 48 kHz loop offset and length for tagged Opus. The split output contained a 529,200-frame opening and a 352,800-frame repeat. The folded WAV marked its repeat at frame 352,800 and the tagged WAV at frame 529,200. The final binaries reproduced all four smoke files byte for byte.

## Review

The simplify pass removed avoidable PCM allocations, reduced the time separate job JSON is held, and simplified folding and peak measurement over the isolated repeat buffer. A unit test checks that the opening fade reaches the repeat smoothly.

Independent high-mode review found that Strudel patterns such as `stepBind` can change the body pitches when queried from a different origin. Both passes now use one selected schedule and preserve its values, times and seeds through each chunk. Multi-chunk planner tests and a real headless `stepBind` regression cover the correction. The review also identified stale host response documentation, which was updated together with the frame summary contract. Re-review found no outstanding issues.
