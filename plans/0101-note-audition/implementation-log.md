# Implementation and validation

Task 101. Testing and review base `e60c4863cacf0d0d80eca2996ef3967f75b9916c`. Moved the checkbox from the toolbar to the status bar after the first capture showed crowding at 1280×800. Chromium verified the resulting placement at 1280×800 and 720×480.

## Implementation

- The preference persists with the Vim preference and defaults to off for existing files. Whole-snapshot writes recover from failed startup reads without overwriting a saved field.
- The editor resolves typed pitch atoms and a fixed voice from its current transaction. The preview owns a bounded synth voice on the Web Audio master graph and cancels it on playback or editor changes.
- Engine handoffs, memo playback and recording block audition. Mouse and keyboard checkbox gestures leave a stopped Rust playback session closed.
- The connected audition voice applies the default playback outline to its unsaved token. Clearing the audition leaves the song's evaluated highlight locations available.

## Review

The simplify pass reused the running preview graph and the existing recording and memo subscriptions. It found and verified the delayed engine handoff case.

Independent high-mode review covered the task diff through `35ee9613`, including the checkbox placement change. Findings addressed preference read recovery, long-document parsing, valid single-argument call syntax, arrangement controls, common shadowed names, and checkbox gestures after Rust Stop. An independent rerun at `d22cc0b5` passed 71 focused tests and verified all six original repros. Task 102 records the remaining custom JavaScript binding cases identified by that rerun.

Review of the requested outline found that a stashed song retained its old audition mark and that typing a separator cleared the mark before the note ended. The fixes clear restored states and map the outlined span through edits that leave its token intact. The boundary check uses Strudel's atom grammar, including characters such as the period in `g3.5`.

## Checks

- Bun frozen install, frontend build, typecheck and Prettier passed.
- The frontend suite passed 3,256 tests with the host reproducibility check enabled. The command excluded `target/**` because that directory contains an archived task source tree.
- The Rust workspace suite passed 2,153 tests with 6 opt-in tests ignored. Watcher and Unix socket tests required the normal local environment. Rust formatting, Clippy, and the focused headless-host binary tests passed.
- Chromium used real Web Audio and test project IPC. It measured a sine G3 at the master output, verified the second voice and rapid completion, cutoff on Play, silence during song playback and preference persistence. The Rust-configured run mocked native IPC and confirmed mouse and Space toggles after Stop issued no new playback-start call.
- Chromium measured the audition outline as the same color and solid 2 px width as the live playback outline. The mark covered only `g3` and cleared at natural end and when Play was pressed.
- An isolated macOS debug bundle built and launched before the outline addition. Its fixture opened and the native checkbox toggled. Native listening and the WKWebView outline remain manual review checks.
