# 0123 step 0: re-verification on main f7861f5

The issue stands, and it has grown. `src/App.svelte` is 330 lines and now also holds an instrument browser modal and a settings modal.

## App.svelte

- **Modal state:**
  - `newSongOpen` at :47;
  - `instruments {root, selection, opener}` at :48-64. Closing moves focus to the editor if applied, else to the opener. An effect drops it when the project root changes;
  - `settings {opener}` at :68, :90-100. Closing focuses the opener if it is still connected;
  - `exportDialog {song, controller}` at :69-72 and :108-109. ExportAudio's `onclose` (:281) nulls it, and the component disposes the controller on unmount;
  - the NewProject modal follows `projectActions.folder`.
- **Flags:** `projectsReady` (:65), `nativeMenuReady` (:66), and `quitting` (:67). `quitting` is a plain `let`, read synchronously inside closures.
- **Menu availability:** `exportAvailable = projectsReady && project && active` (:74-76). An effect calls `setExportMenuAvailable` only once `nativeMenuReady` is set (:78-80).
- **The gate:** `actionBlocked() = quitting || exportDialog || instruments || settings` (:82-84). Each caller adds its own conditions:
  - `runProjectAction` adds `projectsReady`. Busy is dropped inside `projectActions.run`.
  - `openSettings` adds `!isBusy()` but does not check `projectsReady`.
  - The `exportAudio` menu item adds `exportAvailable && !isBusy()`.
  - The `newSong` shortcut needs a project and no NewProject naming. It ignores busy and ready.
  - Welcome `onrecent` (:224) and `openInstruments` (:53) are not gated at all.
- **`onAppQuit` (:113-133):**
  - a second Quit while quitting replies `false`;
  - otherwise `quitting = true`, then `controller.requestQuit()` if a dialog is open, then `session.guardShutdown()`;
  - then `replyToAppQuit(id, approved)` in a nested finally, then `controller.finishQuit()` and `quitting = false`.
- **Listener registration (onMount :136-167):**
  - `session.init(canClose)`, where `canClose = !quitting && (exportDialog?.controller.canClose() ?? true)`;
  - an early return if destroyed or no projects are available, else `projectsReady = true`;
  - `onProjectMenu`, re-checking destroyed, then `nativeMenuReady`;
  - `listenForAppQuit`, also re-checking destroyed.
- **Window close** is in `session.ts:1305-1325`, through `api.onCloseRequested`. The close is prevented if:
  - `canCloseWindow()` is false; or
  - `guardShutdown()` returns false; or
  - `canCloseWindow()` is false on a second check, which catches an export started while saving.

  `guardShutdown` (:1123-1133) runs one unsaved decision at a time.
- **Each Tauri adapter has exactly one caller, App.svelte:**
  - `listenForAppQuit` and `replyToAppQuit` (app-quit.ts);
  - `onProjectMenu` (project/api.ts:140);
  - `setExportMenuAvailable` (export/api.ts:83).
- **Tests.** `export-dialog.test.ts` covers the controller's quit states, and `session.test.ts:2508-2600` covers the window-close guard. Nothing tests:
  - the quit ordering;
  - the gate;
  - a menu action during quit or behind a modal;
  - the menu-availability effect;
  - the `canCloseWindow` closure.

## Behaviour that must stay identical

- **Native menu.** New, Open and Settings are always enabled. Export Audio starts disabled and follows only `exportAvailable`. Menu items are never disabled for a modal, a quit or a busy action; those actions are silently ignored.
- **Shortcuts** (Cmd/Ctrl-O and -N). The key is always `preventDefault`ed. Each action keeps its own preconditions, listed above.
- **Quit outcomes:**
  - a second Quit replies false;
  - a busy export, or an unacknowledged result, shows the in-dialog ask, waiting or result UI;
  - a dialog with its folder picker open refuses;
  - the unsaved prompt comes next. Cancel replies false; save or discard replies true;
  - `finishQuit` always runs, and `quitting` is always cleared, even if the reply throws.
- **Window close** is blocked silently (no prompt) while quitting, while an export is not idle, or while a quit is pending. Otherwise the unsaved prompt runs, then the export state is checked again.
- **Focus.** Settings returns to its opener. The instrument browser returns to the editor or its opener. Export returns to the element that was focused before.

## Window close and Quit differ today (kept as is)

- (a) **During an export:** window close is a silent refusal; Quit offers wait or cancel.
- (b) **With an unacknowledged export result:** window close is allowed; Quit asks you to acknowledge it first.
- (c) **The second check:** window close checks the export again after the unsaved prompt; Quit relies on `settling` and the native refusal instead.
- (d) **Modals and busy state:** neither path checks the instrument browser or settings modals, or a busy action.

Unifying any of these changes visible behaviour, so they are filed as a follow-up for the user.

## Boundaries

- **#135** owns the Rust reload and exit ordering (`src-tauri/src/lib.rs`, `app_quit.rs`, `QuitState`). #123 does not touch them.
- **#136** owns:
  - `createApp`'s seams;
  - `onGesture`, `prepareNoteAudition` and `resumeAudio`;
  - StatusBar notices and the preview graph.

  #123 takes `session` from `app.session` and leaves `createApp` as it is.
