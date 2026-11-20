# Task 36 execution plan

Implement the [CLI bundle proposal](proposal.md) on `llm/andrew/strudeland/0036-bundle-cli`.
The CLI's Cargo bin target is `strudeland`. The macOS bundle layout and signing are owned by
Tauri's sidecar implementation. The MSI GUI and CLI share package version and installer lifetime.
The CLI's MSI file is the key path for the component that owns the PATH entry.

## Work order

1. [Stage 1](010-packaging.md) wires CLI compilation, Tauri packaging, and artifact verification.
2. [Stage 2](020-macos-install.md) implements the macOS native action and completes docs.

The stages can be edited in parallel. The native action consumes the macOS sidecar path
`Contents/MacOS/strudeland-cli` from stage 1. Use the existing worktree and Cargo target directory.
Run Cargo commands serially. Run the workspace Rust gate, frontend checks affected by package
script changes, and smoke the macOS app bundle and Windows MSI. The manual Installers workflow
can build both packages from the review branch. Developer ID credentials remain confined to
the existing release workflow.

Keep this file, the proposal, and the implementation log. Remove the two numbered stage files
during Finalization. Stop at the review gate for human approval.
