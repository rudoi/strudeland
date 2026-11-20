# File menu design

- Build from Tauri's default menu. Insert the project items before the existing File commands on macOS and Windows. On Ubuntu insert the new File submenu at the start. Keep the default menu's platform IDs and leave both new commands without native accelerators.
- Route the two known menu IDs through a dedicated event to the main webview. Subscribe through the project API after session initialization and release the subscription on component teardown, including when subscription setup resolves late.
- Use an app-owned project action controller for native menu events, the existing Open Project keyboard shortcut, and the welcome screen buttons. Serialize picker and create/open requests and block them while the name form is visible. Use the existing StatusBar and Welcome notices for session failures.
- Present the existing folder/name flow as a keyboard-accessible modal in both app states. Show the selected path, use the folder basename as the initial name, and make Escape and the cancel button equivalent before submission.
- Keep filesystem writes and the unsaved-edit decision in the existing session. Validate the common desktop paths locally and use native platform CI for build coverage where available.
