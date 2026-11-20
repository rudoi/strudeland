# Implementation and validation

Record completed stages, deviations and test results here.

## Native application Quit

Commit `7812fa2` adds the AppKit delegate callback, the webview channel and the shared
export/exit permit. The three focused Rust tests passed. Run the native callback smoke
with the completed webview flow.

## Webview quit decision

The frontend reuses the export dialog and the project session's unsaved check. The quit
request and the export result can arrive in either order. The dialog keeps the outcome
available until the user acknowledges it and preserves that outcome if the user cancels
the unsaved prompt. The interim transport used a channel. The final transport uses a
targeted Tauri event so an individual delivery failure cannot block later event indices.

## macOS smoke

An isolated macOS 26.3 debug bundle used a synthetic project and a dedicated CLI socket.
Cmd-Q and the AppKit Quit menu both reached the dialog during planning. Cancel Quit left
the export running. Wait showed a planning timeout before Continue Quitting, and cancelling
the unsaved prompt kept that failure visible. A bounded fixture then produced a stereo
44.1 kHz 24-bit WAV. The dialog showed the written path before Continue Quitting. Save all
persisted the edited song, closed the app, and removed the control socket.
The final debug bundle built and answered its isolated control socket. Desktop automation
timed out when inspecting the Dock. The minimized-window path was reviewed against Tao's
native restore and focus calls.

## Review fixes

The independent high-thoroughness review identified four issues. The fixes restore a
minimized window on Quit, preserve AppKit acknowledgements for system-wide termination,
retain a cancelled replacement as a visible result, and protect the completed result
when it arrives before the Quit event. The Apple event descriptor test covers system
quit reasons without logging out or shutting down the development machine.
The lockfile records the Apple Event Manager bindings enabled by the existing Foundation
dependency. The four focused native tests and the frontend regression tests passed.

## Final checks

- Frontend build, typecheck, and lint passed. Bun ran 3,216 passing tests with one opt-in host
  rebuild skipped. The test invocation excludes the shared Cargo output tree, which contains
  earlier source snapshots.
- The locked Rust workspace suite passed 2,127 tests with six ignored, including the four
  native Quit tests and the partial-publication test.
- Locked workspace Clippy and Rust format checks passed after making the existing AppKit
  callback type explicit.
