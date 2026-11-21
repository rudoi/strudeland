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
