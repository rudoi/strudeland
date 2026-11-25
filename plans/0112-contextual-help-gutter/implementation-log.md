# Contextual help implementation log

The initial base was `8312dd9`. The branch was rebased to `274a215` on `origin/main` when the song-header refactor landed. Git range-diff verified all 15 reviewed patches replayed unchanged. Work stayed in the existing task worktree.

## Changes

- `2a5c04e` adds the shared optional help binding, per-control accessible descriptions, and pointer/focus handling.
- `3deef99` integrates the status bar and Settings with native menu routing, a centered circular gear, the existing audition preference, and offline license information.
- `a63184d` applies help across the control inventory and integrates the CodeMirror content, suggestions, and slider.
- `d56df5f` keeps help correct when action state changes, active controls disappear, and modal focus changes.
- `e6bd6eb` consolidates menu guards and recording copy, simplifies help registration, and keeps the shared-definition button focused by updating it in place.
- `b3ef293` fixes hover re-entry during the exit grace period and reflows the footer before zoom can squeeze its text.
- `d17e597` makes note resize help accurate for short and long notes.

## Browser and accessibility checks

The production Chromium smoke used a fixture project and mocked desktop IPC. At 1280 × 800 and 720 × 480, the roll’s help changed without moving the editor or piano roll. Tab reached Edit shared definition, Enter switched to Back to selection with focus retained, and a second Enter returned to the original view. Disabled Previous bars showed its reason. CodeMirror exposed the selected suggestion’s existing documentation and the new help together. Moving through suggestions and closing the menu updated the gutter. An editor test also changed a real slider’s value and checked that the source edit, accessible name, and description cleanup worked.

Settings verified the same gear and menu entry point, a 28 × 28 px gear button with zero icon-center offset, keyboard preference changes and persistence, no audio resume on Settings-only gestures, Escape and focus restoration, modal guards, and offline license disclosures. The footer measured 44.2 px at the normal size and 59.8 px at the minimum size with no notice shown. Zoom and added text spacing preserved height as help changed in the tested controls. Forced colors and long notice checks left controls and notices accessible.

New Project, Export Audio and Instrument Browser smoke passed at 1280 × 800, 720 × 480 and 430 × 300, including fixed help slots, focus, scrolling, and state changes. Timeline and recording smoke verified a label/input pair, the keyboard menu, disabled hover, and the clip control. Browser tests inspected role, name, focus and accessible descriptions. Speech on VoiceOver requires a separate check.

## Quality

The simplify pass inspected the complete branch diff and consolidated the live target registry, modal-action guard, and recording copy, and eliminated the piano-roll definition button remount. The independent high review found hover re-entry, zoom-driven layout changes, and wording that overstated pointer resize support for short piano-roll notes. The fixes are listed above.

## Verification

- The static Svelte inventory found a help prop or binding on all 104 app control declarations checked. CodeMirror controls are registered by the editor extension.
- Frontend build, typecheck, lint, and tests passed. The Bun test run excluded prior task artifacts under `target/`. After the rebase, it reported 3,532 passed, with the opt-in host bundle rebuild test skipped. The production Chromium roll smoke also passed again on the rebased head.
- A disposable native macOS WKWebView ran the production build at 720 × 480 with mocked IPC. It verified the centered gear, focus-driven workspace help, blank inactive workspace help, the audition toggle and its updated accessible description, and stable dialog size while help changed. The WKWebView snapshot was inspected.
- The default Rust workspace test invocation hit local socket and watcher permissions in the sandbox. With those permissions, six watcher tests stalled in the parallel run. Each of those six passed in isolation using the same test binary and configured Cargo target. The remaining workspace tests and doctests passed. The initial Rust matrix totaled 2,201 passed and seven ignored. Workspace Clippy and format checks passed.
- On the rebased head, workspace Clippy and format checks passed again, along with all 207 selected desktop app unit tests. The six isolated watcher cases cover unchanged code.
- The native menu code compiled and its event was exercised through the desktop event bridge. Direct OS-menu activation, VoiceOver speech, and Windows and Linux WebViews remain unverified locally.
