# File menu project commands

## Goal

Make it possible to find and use **File → New Project…** and **File → Open Project…** from the welcome screen or while editing a song.

## Scope

- Add both commands to the native desktop File menu for macOS, Windows, and Ubuntu.
- Use the existing project folder picker for opening a project. To create a project, choose a folder and confirm a project name, prefilled from the folder name.
- Keep project changes in the current window and preserve the session's unsaved-edits prompt. Keep the welcome screen buttons and the existing platform shortcuts available.
- Additional File commands (Open Recent, Save, and New Song) are tracked in task 96. Multiple project windows and browser project support are outside this proposal.

## Architecture

- Extend desktop menu setup in [`src-tauri/src/lib.rs`](../../src-tauri/src/lib.rs). Add the two commands to the existing File submenu where present, or add a File submenu where the default menu has none. Keep the existing platform menu items and behavior.
- Forward the two recognized commands to the main webview with a dedicated project-menu event. Use [`src/lib/project/api.ts`](../../src/lib/project/api.ts) for the typed listener and have [`src/App.svelte`](../../src/App.svelte) manage its lifecycle alongside session initialization.
- Send Open Project to `session.openProject()`. Extract the folder/name/create interaction from [`src/components/Welcome.svelte`](../../src/components/Welcome.svelte) so the welcome button and the native New Project command share one flow owned by the app shell.

## Technical design

- Reuse `pickNewProjectFolder()`, `defaultProjectName()`, and `createProject()` from the [project session](../../src/lib/project/session.ts) and [project UI helpers](../../src/lib/project/ui.ts). Keep the file creation, unsaved-edit handling, and project switching in the session. Make picker, name-form, and unsaved-dialog cancellation leave the current project accessible. Make picker and creation errors visible from both the welcome screen and the editor.
- Keep the current Command/Ctrl+O shortcut owner in the webview. Leave accelerator assignment for a future shortcut change so one keypress opens one picker. Guard repeated or overlapping project commands while a picker or creation form is active.
- Verify native menu construction against the locked Tauri version. Test command routing and cancellation together with the existing project-session coverage. Smoke test both commands on macOS from the welcome screen and with an open project, including cancellation with unsaved edits. Record Windows and Ubuntu verification separately for platforms available during testing.

## Dependencies

- The current Tauri menu and event APIs, dialog plugin, main webview, and project session.
- The native desktop app for the menu smoke test.

## Risks

- Tauri's default menu differs by platform. Verify that adding File commands preserves platform-provided controls, including macOS window and close behavior.
- A native menu event can arrive during a project transition or webview reload. Ensure the listener is released on teardown and that overlapping actions cannot create duplicate dialogs or skip the session guard.

## Open questions

The proposal keeps the existing shortcuts and uses the same folder-first creation flow on all supported desktop platforms. Refinement can adjust either choice before implementation.
