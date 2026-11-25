# Caret-driven piano roll implementation log

## Start and contracts

Keitora #111 proceeded with user-authorized automatic advancement through entry to the review
gate. The revised goal makes first-drag editing the contract for the roll. The task commits on
the existing user-created checkout.

The proposal was split into editor interaction, authored grids, and source-backed event edits.
Verified their links and contracts, reconciled the requirements, and committed the execution
sequence. The caret resolver change in `05fb9b3` passed all 20 source tests, including lexical
caret ownership, alias and effect boundaries, and reopening an existing edit wrapper.

## Validation evidence

The proof requirements in the [proposal](proposal.md) apply to the integrated feature.
Record additional results below as they complete.

## Integrated implementation

- The caret opens the nearest proven pitch value and keeps explicit ranges authoritative. Cached
  source and lexical-use analysis preserve the active scope across local caret motion. Shared
  definitions retain an explicit navigation path.
- The Luoyuan adapter recognizes the local legacy consumer by its operations and lexical
  bindings. The reusable `noteGrid` helper shares exact row decoding. In-place output keeps
  grid spelling, glide controls and source formatting. A horizontal drag uses authored slot
  spacing against the existing meter ruler.
- A generated-note or In song drag writes a scoped event edit in one source transaction.
  Runtime haps retain their exact spans and controls. The final-output operation uses the
  written occurrence where verified and the inspected absolute occurrence for independently
  clocked or outer-transformed material. Captured pitches require evidence that the lane
  varies. Inspection reports a stale deterministic selector without deleting the current music.
- CodeMirror commits all patches as one undo item. It reveals the edited source, retains roll
  focus, and marks the affected notes and written spans with reduced-motion support.

## Test and review iteration

- The Chromium app smoke ran against a local project transport stand-in and the real host JavaScript
  inspection entry. Direct, repeated generated, and arranged gestures wrote before fresh inspection
  returned. It verified source scrolling, target stability, roll focus, note and source highlights,
  an undo, a horizontal edit, and reduced motion. A separate smoke used the local Luoyuan script
  in memory and changed only its D5 pitch token while retaining the glide suffix.
- The render page and native host produced identical mixed-meter plans for a synthetic song
  using both a local event edit and a final-output edit, in clip, tagged, intro, and folded
  loop layouts. The local Luoyuan probe verified the corresponding two-layer pitch change and
  the next glide's expected bend.
- The first workspace test sweep picked up archives under the shared Cargo target. The suite was
  scoped to the repository's `tests` directory. It caught raw CSS pulse timings, which now share
  a motion token. Local socket and FSEvents tests required an elevated retry; all 213 app tests
  passed on that retry.
- The simplify and independent correctness passes covered the full task diff across editor,
  source resolver, grid, runtime, host and transport. Regressions retained during those passes
  include disconnected note fragments, multiple continuous cuts, wrapped shorthand and ASI,
  carry-in movement, duplicate note focus, source reveal anchors, re-registration, generated
  pitch shape, changed seeds and stale selectors. The final gate reruns across the integrated tree.

## Review candidate

The final code snapshot is `6670cf2`. Local validation passed:

- `HOST_BUILD_CHECK=1 bun test ./tests` ran 3,487 tests with zero failures. The frontend
  build, Svelte and TypeScript checks, and Prettier checks passed.
- `cargo test --workspace` ran 2,201 passing Rust tests with seven explicitly ignored
  benchmarks or reference probes. Rustdoc, workspace Clippy with warnings denied, and
  `cargo fmt --all --check` passed.
- Standard mixed-meter host parity and a saved-event-edit probe each matched four render plans.
  The built frontend smoke verified direct, generated and arranged edits, same-scope
  consecutive gestures, auditioning, history, source reveal and reduced motion. The local
  Luoyuan grid and a unison keyboard-focus case were verified separately.
- Independent high-thoroughness reviews covered the editor and the runtime/host contracts.
  Rechecked fixes for the reported cases, including exact fragments and timing across query
  partitions, direct-grid glide feedback, seeded variation, and retained focused voices.
- Updated the README and AGENTS.md and checked the plan and repository links. Existing
  Keitora #110 tracks general direct cross-bar serialization.

## Final review

The signed desktop build was launched from the reviewed branch. The user tried it, accepted the
piano roll result, and authorized finalization and merge. This finalization commit records the
review and removes the two temporary execution files named in the entry point.

Keitora #112 tracks the requested contextual help gutter for interactive controls throughout
Strudeland. Its acceptance cases start with the piano roll’s shared-definition and return
actions, then cover hover, keyboard focus, and a common help mechanism across control types.
