# Task 36 implementation log

- Base `196e59d` on `origin/main`. The implementation sequence is in
  [000-prompt.md](000-prompt.md).
- `b8db241` verifies the upstream macOS and MSI packaging contracts. The pinned MSI template's
  PATH-labeled feature has no Environment row. Task 36 uses Tauri's WiX fragment support to keep
  the CLI in a stable Program Files directory when the GUI path changes.
- `d047cea` records the hardened-runtime refinement. A hardened ad-hoc signature using the
  app's existing entitlements reproduced V8's CodeRange allocation abort. The packaged macOS
  CLI enables V8 interpreter mode, and the DMG verification renders an offline fixture.
- `75fd8eb` implements the shared build, DMG sidecar, MSI component, and payload validation.
  `4910931` adds native menu installation and five filesystem tests.
- Frontend build, typecheck, and lint passed. The workspace's `tests/` suite passed with 3,185
  tests and one opt-in check skipped. The unscoped Bun command also discovered historical
  source copies under the shared Cargo target directory and was stopped.
- `50f4664` fixes the Tauri WiX runner's environment filter and keeps debug launch paths tied
  to Cargo metadata. `286d541` validates the actual arm64 Mach-O or x64 PE before staging, and
  rechecks the staged executable in the split signing path.
- The simplify pass tied the macOS install destination to the confirmation dialog,
  reused validated MSI metadata for extraction checks, and fixed debug bundle lookup.
  Independent high review found the WiX environment and architecture issues. The reviewer
  rechecked both fixes and reported no remaining findings in those paths.
- The ad-hoc Apple Silicon DMG passed checksum and nested signature checks, reported
  `strudeland 0.1.0`, ran `docs --list`, and rendered a two-second WAV from the mounted image.
  A temporary symlink to the packaged CLI reached the running app and returned `state` over an
  isolated socket and captured the app window. Developer ID signing and native Windows installation require their
  respective environments.
- The final locked Rust workspace test run passed with 2,113 tests and six opt-in tests
  ignored. Workspace Clippy with warnings denied and the Rust format check passed.
- Hosted PR checks passed on macOS, Windows and Ubuntu. The manual installer workflow
  built and verified the Apple Silicon DMG, Windows x64 MSI, and Ubuntu amd64 DEB. The Windows
  verifier checked the MSI PATH registration and ran the CLI from the extracted payload.
  [Hosted installer run](https://github.com/rudoi/strudeland/actions/runs/44678740241)
