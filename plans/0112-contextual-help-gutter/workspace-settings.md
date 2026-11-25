# Workspace and Settings

The current app shell and footer are in [App.svelte](../../src/App.svelte) and [StatusBar.svelte](../../src/components/StatusBar.svelte). The workspace gets one full-width status bar with a 12 px Help slot. Keep notices and their actions, editor toggles and the master meter available. Reserve lines outside the active message. Dialogs have a fixed Help slot inside their modal scope with scrollable content above it.

The new Settings gear is a true circle with one equal inline and block dimension and centered icon alignment. Its accessible name is Settings. Settings is available on Welcome and in the workspace. A modal holds the existing Audition notes checkbox and an About & licenses section. Keep the preferences store and gesture exclusion. Move version and Strudel details here alongside the copyright, license, redistribution, warranty and source information described in the [proposal](proposal.md).

Extend the existing native menu bridge in [project_menu.rs](../../src-tauri/src/project_menu.rs) and [project API](../../src/lib/project/api.ts). Use the app menu and Command-comma on macOS. Use File on Windows and Linux. Gear and menu requests share an opener in App. Prevent overlapping modals, use a single native shortcut path, and restore focus on closure. Keep the existing project menu shortcuts, app quit behavior and editor gestures.

Verify the circular button geometry and icon offsets, legal and version information, persisted preference, menu routing, help in each modal, and workspace and footer sizes at normal and minimum layouts. Verify both keyboard and pointer use.

## Integration points

Use `App.svelte` as the composition root for the modal guard and native activation. Use `StatusBar.svelte` for the gear and persistent status content. The existing `onProjectMenu` callback type already covers export alongside project actions and can carry Settings. Nest the shared `Help` component inside each actual open dialog and use `compact` only in workspace chrome. Keep the master meter mounted while the Help text changes.
