# Stage 1. Package the CLI

Goal. Include the matching native CLI in the DMG and MSI.

Prerequisites. The CLI Cargo package, the Tauri release and local build paths, and native
compiler tools.

Implementation. Use a shared Bun entry point to collect the locked Cargo build's bin artifact
and bind it to the macOS sidecar overlay or Windows WiX source path. Wire the unsigned
installer job, the split signed macOS job, and the local signed debug command. Check payloads
and version from the resulting installer.

Validation. Test argument and platform selection and artifact parsing. Run an unsigned macOS
DMG build and Windows MSI build, verify each packaged CLI, and inspect Windows installation,
upgrade, and removal if a desktop is available.

Commit message. `feat(packaging): ship the Strudeland CLI with desktop installers`
