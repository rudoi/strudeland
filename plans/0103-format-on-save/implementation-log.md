# Implementation log

- Components in `fd7eb8c` and `a9f5f67`. Added an opt-in preference that preserves old files
  and loads independently of Vim changes. The pure parser-based formatter emits localized
  edits and verifies both mini structure and surrounding JavaScript. Tests cover two-row
  spacing, note timing and source starts, existing slow sequences, unsupported forms, and
  repeat saves.
- Integration in `f75dde9`. Kept the preference and captured source with the save request
  through file moves and watcher echoes. Session tests cover explicit Save, Save and Play,
  stashed control saves, preexisting highlights, undo, incomplete code, and failed writes.
  App tests exercise Vim write with the hydrated preference.
- Scope clarification during integration. A slow sequence gets formatted when all of its
  bar bodies are eligible. Task #104 tracks the vendored leaf end offset at a newline.
- Rebased onto `2f3fd7c` after task #101 reached main. Reused the shared preference store
  and kept note audition and format-on-save independent, including both persisted values.
- Pre-push quality fixes in `289e12b`. Reused the CodeMirror transaction's resulting
  document as the save snapshot and removed redundant preference-fixture defaults.
  Independent review found a slow quote-verification fallback, inconsistent display
  width for wide atoms, and missing coverage when a save waits for a failed move.
  Added a linear Acorn boundary check and assertions for both safe and ambiguous
  boundaries, restricted layout to predictable ASCII atom columns, and exercised
  both pending-move branches. The 512-boundary stress case that took roughly 3.6 s
  validated in 32–48 ms in local Bun probes after the fix.
- On base `2f3fd7c`, the locked Rust workspace tests passed (2,156 passed and 6 ignored),
  along with workspace Clippy and rustfmt. A frozen Bun install, build, typecheck,
  tests, and Prettier also ran on the rebased branch.
- Isolated macOS desktop smoke on the shared Cargo target. The default save preserved
  the flat source byte for byte. After restarting with the preference enabled in the
  separate test profile, the real app saved two 16-step rows while playback continued.
  The native control endpoint reported 16 identical note onsets and values before and
  after re-evaluation. Inspected the app's before and after screenshot captures.
