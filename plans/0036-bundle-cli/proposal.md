# Bundle the Strudeland command line tool

## Goal

Users who install the macOS app or Windows x64 MSI can run the version-matched `strudeland` CLI
without a Rust toolchain. The macOS app offers an **Install command line tool…** action. Windows
installs `strudeland.exe` as part of the MSI and offers terminal access through the installer.

## Scope

- Include the workspace-built CLI in Apple Silicon app bundles and DMGs and the Windows x64
  MSI, covering local builds, manually built installers, and tagged releases.
- Offer user-owned symlink installation on macOS and installer-managed PATH setup on Windows.
  Document terminal setup, upgrades, and removal on each platform.
- Keep Cargo source installation supported. The Windows NSIS and Ubuntu DEB packaging paths,
  shell configuration edits, and command or control-protocol changes are outside this task.

## Architecture

- Use Tauri packaging overlays for the DMG and MSI. On macOS, use `externalBin` to package
  `strudeland-cli` beside the app executable. Stage a target-suffixed binary under an ignored
  directory and let Tauri sign it with the app. [Tauri external binaries](https://v2.tauri.app/develop/sidecar/)
- On Windows, use Tauri's WiX fragment support to install `strudeland.exe` and the PATH entry as
  one stable component under Program Files. The pinned template puts external binaries in the
  configurable GUI install directory. Its PATH-labeled feature contains component references
  and leaves PATH authoring to the installer. A fragment lets the CLI stay in a protected path
  while the GUI retains its install options, executable name, and upgrade identity.
  [Tauri Windows installer](https://v2.tauri.app/distribute/windows-installer/#customizing-the-wix-installer)
  [Pinned WiX template](https://raw.githubusercontent.com/tauri-apps/tauri/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/windows/msi/main.wxs)
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
drives local macOS and Windows MSI packaging. Route [the stable debug signing helper](../../scripts/app-signed.ts),
[manual DMG and MSI builds](../../.github/workflows/installers.yml), and
[the split macOS release build](../../.github/workflows/release.yml) through the same staging logic.
The Windows release already reuses the manual MSI build job.
Preserve the signing helper's open behavior and the release workflow's signing and notarization
steps. Fail the build if Cargo or staging fails so an old staged file cannot be silently packaged.
The macOS sidecar uses V8's interpreter mode under hardened runtime and the existing signing
entitlements. The ordinary source build and Windows MSI keep the default V8 mode.

Extend [installer verification](../../scripts/stage-installer.sh) to inspect the mounted DMG,
check the nested executable's architecture and signature, and compare its `--version` with the
release version. On Windows, extract the actual MSI payload and run its x64 CLI with the same
version check. Run an offline CLI smoke command from both packages and verify the GUI and CLI
executables are distinct. Render a corpus fixture from the signed macOS sidecar to exercise V8.

### Installing on macOS

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

### Installing on Windows

Use the existing per-machine MSI scope. Install the CLI under Program Files in `Strudeland CLI`
independently of the selected GUI location. The WiX component appends that directory to machine
PATH for newly opened terminals and removes its entry at uninstall. Keep a stable component GUID
and file key path across upgrades. The component references the exact Cargo-built executable via
an environment variable set by the shared packaging command.
[WiX Environment element](https://docs.firegiant.com/wix3/xsd/wix/environment/)

## Dependencies

- The CLI and local control protocol already ship as source. The embedded CLI runs the same
  binary with the same control-socket discovery and offline commands.
- The existing installer jobs provide the native Rust toolchain, WiX for the MSI, and the
  macOS signing and notarization steps. Verify that each runner has the CLI's V8 and encoder
  prerequisites, including CMake and a native compiler.
- Rust's Unix symlink API and Tauri's existing menu, path, and dialog APIs cover installation.

## Risks and validation

- A stale artifact or mismatched target could produce a CLI that launches on the build host
  alone. Check target/profile selection and walk the packaged DMG on Apple Silicon, including
  the ad-hoc signing path and the signed release path when authorized.
- On Windows, test a clean MSI install, upgrade from the currently shipped MSI, custom install
  location, and uninstall. Verify app shortcuts launch the GUI and a fresh PowerShell or cmd
  session resolves `strudeland` to the installed CLI. Check that PATH retains unrelated
  entries and drops the MSI-managed entry on removal.
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

## Refinement decisions

- Use the native macOS app menu and `~/.local/bin` with explicit PATH instructions.
- Use the existing Tauri WiX template with a fragment and verify the resulting MSI component,
  PATH entry, and installed CLI against the published MSI upgrade path.

**Recommended complexity.** Medium. Keep a reference and interface pass before sequencing.
The proposal covers shared CLI staging and the platform-specific installation steps.
