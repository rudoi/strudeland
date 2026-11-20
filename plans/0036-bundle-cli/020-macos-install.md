# Stage 2. Install the macOS CLI link

Goal. Expose the bundled CLI under the existing terminal command name.

Prerequisites. The packaged CLI sits alongside the macOS GUI under `Contents/MacOS`.

Implementation. Extend the native macOS app menu and wire the install dialog to a Rust
symlink helper. The helper refuses conflicts and transient app locations. Document terminal
setup for both packaged platforms.

Validation. Test install, repeat and error cases. Smoke the packaged app, open the installer
dialog and verify the linked CLI against the running app where the desktop permits.

Commit message. `feat(cli): add the macOS command line install action`
