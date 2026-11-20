# Bundle the Strudeland command line tool

## Goal

A user who installs the macOS app can run the version-matched `strudeland` CLI without a Rust
toolchain. The app's **Install command line tool…** menu action creates a user-owned symlink
and explains how to invoke the command from a terminal.

## Scope

- Include the workspace-built CLI in Apple Silicon app bundles and DMGs made by the local
  packaging command, the unsigned installer workflow, and the signed release workflow.
- Offer installation from a packaged app without administrator privileges. Document the link
  location, shell setup, app upgrades and moves, and how to remove the link.
- Keep Cargo source installation supported. Native CLI installation and PATH management on
  Windows and Linux are outside this macOS task, as are shell configuration edits and updates
  to the command line or control protocol.

## Architecture

- Use Tauri's [external binary configuration](https://v2.tauri.app/develop/sidecar/) in a
  macOS packaging overlay for [the app config](../../src-tauri/tauri.conf.json). Stage the
  target-suffixed input under an ignored directory. Give the sidecar the packaged basename
  `strudeland-cli` so its executable remains distinct from the app binary, and expose it to
  terminals under the existing command name `strudeland`.
- Keep the overlay out of plain Cargo checks, `tauri dev`, and the release workflow's
  `--no-bundle` compile. Tauri's Cargo build helper copies configured external binaries
  during app compilation, so a bundle-only hook cannot supply inputs for an ordinary combined
  build. Prepare the sidecar before a combined build applies the overlay. For the split signed
  build, prepare both binaries before importing the signing identity, then apply the same overlay
  in `tauri bundle`.
- Register the action in the native macOS app menu from
  [the Tauri app setup](../../src-tauri/src/lib.rs). Extend the standard menu and use the
  initialized dialog plugin for confirmation and results. Resolve the source from the running
  app's `Contents/MacOS` directory and keep symlink creation in Rust.

## Technical design

### Building and packaging

Build the existing `strudeland` binary from
[the CLI package](../../crates/strudeland-cli/Cargo.toml) with Cargo's lockfile, using the same
checkout and the app bundle's target and debug or release profile. Resolve the output from Cargo
and reuse its configured target directory. A shared Bun entry point prepares the sidecar and
drives local macOS packaging. Route [the stable debug signing helper](../../scripts/app-signed.ts),
[manual DMG builds](../../.github/workflows/installers.yml), and
[the split release build](../../.github/workflows/release.yml) through the same staging logic.
Preserve the signing helper's open behavior and the release workflow's signing and notarization
steps. Fail the build if Cargo or staging fails so an old staged file cannot be silently packaged.

Extend [installer verification](../../scripts/stage-installer.sh) to inspect the mounted DMG,
check the nested executable's architecture and signature, and compare its `--version` with the
release version. Run an offline CLI smoke command from the bundle. Verify the identified app
executable is distinct from the CLI.

### Installing from the app

Show the menu entry when running from a macOS app bundle with the CLI present. The dialog names
the destination, `~/.local/bin/strudeland`, and lets the user cancel before writing. Encourage
installation after copying the app to Applications and opening that copy. Reject a transient
launch or a mounted installer as the source so the link can survive a restart or eject.

Create the user-local directory as needed and use symlink creation that fails if the destination
already exists. An existing symlink to the same packaged path is success. Preserve other links,
regular files, and directories at the destination and explain the conflict. Treat a missing source
or filesystem error as failure and report the affected path. Leave shell startup files alone.
Report the full destination and give instructions for adding `~/.local/bin` to the terminal's
PATH. A replacement app installed at the same path keeps the link usable. A moved app requires
the user to remove the old link and run the action from the new location.

## Dependencies

- The CLI and local control protocol already ship as source. The embedded CLI runs the same
  binary with the same control-socket discovery and offline commands.
- The existing macOS installer job has the native Rust toolchain and signing and notarization
  steps. The CLI brings the existing V8 and encoder build dependencies, including CMake and a
  C compiler. Check both installer entry points have those build prerequisites.
- Rust's Unix symlink API and Tauri's existing menu, path, and dialog APIs cover installation.

## Risks and validation

- A stale artifact or mismatched target could produce a CLI that launches on the build host
  alone. Check target/profile selection and walk the packaged DMG on Apple Silicon, including
  the ad-hoc signing path and the signed release path when authorized.
- The app may run from a DMG or from an App Translocation path. Verify these launches never
  install a temporary target and that the copied app can install successfully.
- Test link creation and repeat installation in temporary directories. Cover a missing CLI,
  an existing file, a directory, an unrelated or dangling link, and a write failure. Verify
  the cancel path and conflict dialog in the app.
- Exercise the linked command from a fresh terminal with the app closed for `--version`
  and an offline command, then run `strudeland state` after opening the app. Check an in-place
  app replacement keeps the link working.
- Run the workspace Rust tests, format check and Clippy, plus focused packaging-script and
  workflow checks. Check that a clean dev or Cargo build works without a staged sidecar.
  A Developer ID artifact needs the existing release authorization and credentials.

## Open questions for refinement

- The proposal uses `~/.local/bin` with explicit PATH instructions. Confirm that destination
  and the native app menu placement.
- The initial delivery is macOS-only. Windows and Linux currently use the documented Cargo
  installation path.

**Recommended complexity:** Medium. Keep a reference and interface pass before sequencing.
Skip fracture and the separate loss check because one proposal can cover both coherent stages:
producing the bundle and installing from it.
