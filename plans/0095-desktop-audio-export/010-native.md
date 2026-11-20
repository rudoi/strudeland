# Native export

## Goal and prerequisites

Implement the Tauri command contract in [contract.md](contract.md) atop the shared renderer.

## Implementation

Reuse filename construction and project-path validation. Keep the folder selected by the native
picker in app state. Recheck targets before launching one background job and apply atomic
no-replace publication when the preview did not approve replacement. Report any partial write.
Route native menu events and wire the renderer's existing hardened runtime feature on macOS.
Preserve the CLI's existing output behavior.

## Validation

Test names and roles, stale preview, unapproved collisions, split partial writes and a real
synthetic WAV render. Run the focused app, project and renderer checks before integration.

Commit message `feat(export): add native desktop audio export`.
