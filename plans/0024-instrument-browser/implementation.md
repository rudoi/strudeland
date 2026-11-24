# Instrument browser delivery

The [proposal](proposal.md), [browser and editor contract](browser-and-editor.md),
[storage and preview contract](storage-and-preview.md), and [automation contract](automation.md)
describe the implemented behavior.

## Implementation

- The project crate owns validated library creation and fingerprint-checked reading
  and publishing. Tauri renders the selected definition through the Rust engine and
  transfers a two-second WAV to the app's existing audio graph.
- The desktop dialog shows library definitions and project pins, their voices and
  defaults, and the available preview and project actions. Preview is separate from
  song playback. The current preview stops when the modal closes, its selection
  changes, or transport, a memo, or a recording starts.
- Desktop, CLI and MCP use the same conditional editor operation. Apply validates
  the source and range before editing, preserves surrounding syntax, and verifies
  the active song after reading pins. The next evaluation can use the captured
  snapshot before the watcher delivers a pin event.
- The CLI and instrument-only MCP server share the store, project locator, and
  control socket. The server is local over stdio.

## Review

An independent simplification pass reviewed the entire task diff. The implementation
shares the editor's AST comparison, the audio eligibility and graph access, and
preview teardown. An independent correctness pass traced storage, native preview,
browser state, editor transactions, CLI/MCP, and the mirrored protocol.

The review found and the branch corrects partial or non-pattern selections, whole
mini-pattern and constructed-pattern selection, namespace scope, stale evaluation
pins, and races with song deletion and project switching. Regression tests cover those
paths. Fingerprint-guarded apply reads one evaluation snapshot for the version the user
saw in the browser.

## Desktop and automation smoke checks

- Chromium with a mocked native bridge exercised search, keyboard navigation, pinning,
  apply, one-step undo, Escape, and focus return. The browser layout was inspected at
  desktop and 390-pixel viewport widths.
- An isolated macOS development app served the production frontend on localhost.
  It listed project pins, showed both voices and the cutoff default, and entered
  preview playback for the selected kit. Apply made the song dirty, returned focus
  to the editor, and evaluated to six events over one bar. One Undo restored the
  original selection and clean buffer. The native screenshot is retained locally
  with the task's validation captures.
- CLI tests exercised conditional apply and its range output. Real MCP subprocess
  tests covered initialization, discovery, and all five instrument tool calls,
  including the typed socket request for Apply.

## Frontend verification

The production build, typecheck, and Prettier check passed. The frontend test run
passed 3,378 tests with one opt-in host-bundle check skipped. Test discovery excluded
old checkout copies under the local Cargo target directory.

## Rust verification

Locked workspace tests passed with 2,196 tests passing and seven configured ignores.
Locked workspace Clippy passed for all targets with warnings denied. The workspace
Rust formatting check passed.
