# File menu design

Implemented in the desktop app. The local frontend and Rust checks passed, and the macOS menu was confirmed in the isolated debug build.

- On macOS, insert the project items into Tauri's default File submenu and keep the system menu IDs. On Windows and Ubuntu, add a File submenu containing the two project items. Tauri supplies no app menu by default on those platforms, and its predefined Windows Exit would bypass the unsaved-edits prompt. Leave both new commands without native accelerators.
- Route the two known menu IDs through a dedicated event to the main webview. Subscribe through the project API after session initialization and release the subscription on component teardown, including when subscription setup resolves late.
- Use an app-owned project action controller for native menu events, the existing Open Project keyboard shortcut, and the welcome screen buttons. Serialize picker and create/open requests and block them while the name form is visible. Use the existing StatusBar and Welcome notices for session failures.
- Present the existing folder/name flow as a keyboard-accessible modal in both app states. Show the selected path, use the folder basename as the initial name, and make Escape and the cancel button equivalent before submission.
- Keep filesystem writes and the unsaved-edit decision in the existing session. Hold its project-switch guard through folder selection, name entry, and creation so CLI opens cannot interrupt the form. Include a deleted song whose only remaining copy is in the editor in the unsaved-edits prompt. Validate the common desktop paths locally and use native platform CI for build coverage where available.
