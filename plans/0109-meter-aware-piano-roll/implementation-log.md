# Piano roll implementation

The [proposal](proposal.md) and [refinement audit](refinement-audit.md) describe the product
decisions and original probes. The source, inspection and dock sequence is complete.

- The source planner handles the five editing operations inside supported monophonic bars and
  individual pitches in attributable chords. It validates revised source against the exact
  expected events. Shared definitions retain their known uses at the edit point.
- The isolated host supplies generated Expression and In song views. Verified source matches
  are highlighted. Ambiguous arranged notes remain visible with explicit attribution status.
  Unproven expression capture and unsupported authored geometry remain inspection cases.
- Audition uses the verified synth or the stock triangle. It shares preview ownership with
  typed-note audition and preserves the existing transport and recording guards.
- Cross-bar gesture serialization remains gated. One exact two-bar encoding was verified during
  refinement. The general serializer and the effects of alternate meter anchors on a shared
  definition are tracked in Keitora task #110, blocked by this task.

## Quality review

The Simplify pass and independent high-thoroughness review covered the task diff from
`07b072c6`. The second review checked the source, host and desktop fixes.

- Simplify consolidated editor notifications and audition cancellation, and computed the exact
  common denominator once per timing rewrite. It also exposed source and playback parity cases
  involving octave controls, customized pitch calls, and live instrument pins.
- The high-thoroughness review exercised raw-pattern capture, alternate periods, oversized
  replies, authoring grammar, source navigation, overlap rendering and focus. The final fixes
  cover bounded Unicode error text, selection retained through a whole-bar rewrite, partial
  overlap tracks, instrument-preserving audition, meter-anchor retention and linked pitch atoms.
- The macOS workspace run reproduced an existing race on the first instrument-library lock open.
  A standalone run of the competing create path verified a transient `ENOENT`. The fix makes one
  retry through the existing checked directory and non-following open. The concurrent publish
  test continues to require one complete definition and one `already exists` result.

## Browser evidence

The complete frontend ran in Chromium against a sandboxed fixture project with mocked desktop
IPC and the real bundled JavaScript inspection entry. It exercised transformed view, editing
a shared definition, exact repeated keyboard moves, source navigation retaining a 7/8 meter,
explicit stock audition with the automatic preference disabled, minimum-size editor visibility,
and collapse. Captures are under the existing Cargo target directory. A signed macOS listening
check remains for human review.

## Verification

- `HOST_BUILD_CHECK=1 bun test ./tests` passed all 3,430 tests, including regeneration of the
  headless bundle and input manifest. `bun run build`, `bun run check` and `bun run lint` passed.
- The final native inspection suite passed all four tests, including local phase, exact source
  ranges, response and error bounds, and watchdog recovery. The existing concurrent first-publish
  test passed 20 consecutive runs after the lock-open fix.
- `bun run host:parity --mixed-meter` matched Chromium and the native host byte-for-byte for
  clip, tagged loop, intro loop, and folded loop.
- `cargo test --locked --offline --workspace --no-fail-fast` passed 2,201 tests.
  7 tests were explicitly ignored by their suites. The run used macOS socket and FSEvents access.
- `cargo clippy --locked --offline --workspace --all-targets -- -D warnings` and
  `cargo fmt --all --check` passed on the reviewed code.
