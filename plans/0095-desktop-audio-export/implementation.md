# Desktop export implementation and validation

File → Export Audio opens a modal for the active editor snapshot. The frontend previews shared
renderer filenames and reports planning, rendering, encoding and publication. Rust handles
folder selection, replacement approval and off-thread rendering. The same resolved targets
back the preview and publication. Both split outputs are encoded before publication, which
is atomic per file. A partial failure reports completed paths.

## Desktop verification

Tested the ad-hoc signed debug bundle built by `bun run app:bundle app --adhoc --debug` on
Apple Silicon macOS 26.3 (25D125), with system WebKit 21623.2.7.11.6. Signature verification
passed and the bundle had the runtime flag. The File menu drove the real Rust renderer.

Both fixtures used 120 BPM, 4/4 and a one-bar loop. The first used the default project output
folder with a one-bar intro. The second began on the loop and used a different folder chosen
in the native dialog. Preview showed both exact filenames and left a missing `renders/`
folder untouched. Both runs showed encoding progress and an offset of zero seconds.

| Fixture | Opening frames | Repeat frames | Repeat WAV marker |
| --- | ---: | ---: | --- |
| One-bar intro | 176,400 | 88,200 | 0–88,199 |
| Starts on the loop | 88,200 | 88,200 | 0–88,199 |

All four files were stereo 24-bit WAV at 44.1 kHz and initially matched the same CLI exports
byte for byte. Both openings had no sampler chunk. A second export to the alternate folder
listed both existing files in the native prompt. Cancel preserved both files, including
their modification times and file identities. Replace republished both and their decoded
header checks still passed. The preview and completed result were captured with the bundled
CLI and visually inspected at 1280 × 800 logical pixels.

## Automated checks

- The frozen Bun install, production build, all 3,210 tests with a fresh host-bundle check,
  Svelte and TypeScript check, and Prettier passed. Bun discovery excluded archived test trees
  under the shared Cargo target.
- The Rust workspace suite passed 2,121 tests, with six ignored and the timing-sensitive
  `dropout_short` exclusion used by CI. An initial sandboxed attempt could not bind local
  sockets or run the macOS file watchers. The unsandboxed run passed. The app suite passed
  201 tests after the review fixes, including seven export tests. Workspace Clippy and Rust
  formatting passed.

## Quality review

The simplify pass moved editor-text capture off routine menu updates, reused the song's loop
index and shared the file list between preview and result. Independent high-depth review led
to native replacement approval, a window-close check before and after asynchronous saves,
platform-specific dialog parenting and independent project/menu readiness. The reviewer
rechecked the fixes and found no remaining task 95 issues.

Task 99 tracks graceful application Quit during an export. Native UI smoke covered macOS.
Windows and Linux desktop interaction remain outside the recorded smoke run.
