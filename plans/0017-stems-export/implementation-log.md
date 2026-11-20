# Implementation and verification

## Implemented

- Engine capture taps the same offline graph as the reference mix. Orbit routing uses actual mixer behavior when folded channels would otherwise lose a contribution. Shared effects use exact accumulation of finite float terms to preserve quiet contributions through cancellation.
- The pipeline joins every stereo pair with the standard range cuts. It bounds audio buffers, reuses a WAV encoding buffer, and publishes files individually with a manifest last. A reserved directory identity is retained through completion. Unix temporary files are published and cleaned up relative to the open directory. Windows holds the directory open with delete sharing disabled.
- The CLI and desktop export command share the same pipeline. Desktop uses the active editor buffer, keeps the export session through publication, and reports partially written files if a later write fails.

## Verification

- The frontend production build, typecheck, Prettier, and 3,218 tests passed. The local Bun run excluded the ignored `target/` tree, which contains archived checkouts. The optional host rebuild test remained skipped.
- The full locked Rust workspace run on the reviewed code passed all 2,153 enabled tests. Six optional tests were ignored by their existing attributes. The run used native access for the socket and file-watcher tests.
- Workspace Clippy with warnings denied and Rust formatting passed.
- The Rust batch render gate passed `stems-5`, `stems-shared`, `duck`, and `reverb-long`. The existing harness reported its expected shared-channel limitation for `stems-shared`.
- CLI smoke exports in all three modes produced byte-identical reference mixes with 66,150 frames per file. The worst absolute reconstruction error was below 8e-9 across all samples of the synthetic song.
- The Apple Silicon macOS 26.3 debug app smoke used the system WKWebView and an isolated project. The File menu opened the real export dialog, an unsaved editor label appeared in the manifest, and all four WAVs were stereo IEEE float at 44.1 kHz with 11,025 frames. The retry refused the existing destination. Chromium smoke covered desktop and narrow layouts, range selection, and the quit flow with a fake native backend.

## Review

Independent simplify and high-thoroughness correctness passes covered the engine, pipeline, project storage, CLI and desktop interfaces. Review fixes cover folded-channel cancellation, three-scale cancellation, a directory swap between files, and a move during temporary file writing. Follow-up review checked the fixes and the platform guards. Main advanced by an ignore-file update during development. Rebase left the reviewed code diff unchanged.
