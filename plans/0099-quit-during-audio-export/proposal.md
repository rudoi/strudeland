# Application Quit during audio export

## Goal

An ordinary application Quit during desktop audio export gives the user a choice to wait for
the export or cancel the quit request. If the user waits, they can see the final outcome and any
files already written before deciding whether to proceed. Songs with unsaved changes still use
the existing Save, Discard, and Cancel prompt before the app exits.

## Scope

- In scope are macOS AppKit Quit requests, including the application menu, Cmd-Q, and Dock Quit,
  while the desktop export is confirming, planning, rendering, encoding, or publishing.
- Task 95 continues to cover the export dialog and main-window close. Leave their existing
  close rules in place. Force Quit, process termination, export-worker cancellation, and new
  Quit menu items on Windows or Linux are outside this proposal.

## Architecture

- Add a small macOS adapter at app setup in [the native app entry point](../../src-tauri/src/lib.rs).
  Keep the standard Quit menu built by [`project_menu::build`](../../src-tauri/src/project_menu.rs).
  Route AppKit's `applicationShouldTerminate:` callback into the app's quit flow while keeping
  Tao's existing delegate behavior. The pinned Muda menu sends `terminate:` directly, and the
  pinned Tao delegate registers `applicationWillTerminate:`. Wry's `ExitRequested` is emitted for
  a requested Tauri exit or after the last window is destroyed, so it is insufficient as the
  sole interception point. [Apple's termination delegate documentation](https://developer.apple.com/documentation/appkit/nsapplicationdelegate/applicationshouldterminate%28_%3A%29?changes=_1_7&language=objc)
- Defer the decision to the main webview from the native callback and return the AppKit
  cancellation reply synchronously. This keeps the ordinary event loop available for export
  progress and IPC. After approval, enter Tauri's normal exit path so
  [`RunEvent::Exit`](../../src-tauri/src/lib.rs) still cleans up playback, recording, and the
  control socket. [Apple's description of the run loop during termination](https://developer.apple.com/documentation/appkit/nsapplication/terminate%28_%3A%29)
- Use the export dialog footer for the two decisions so the current result stays visible.
  This also avoids adding a second native sheet alongside an open replacement prompt.
- Own the user decision at [the application shell](../../src/App.svelte). Reuse
  [`session.guardUnsaved()`](../../src/lib/project/session.ts) and the result already presented by
  [`ExportAudio.svelte`](../../src/components/ExportAudio.svelte). Keep Tauri IPC at the API
  boundary and the active export's outcome in
  [`createExportDialog`](../../src/lib/export/controller.ts).

## Technical design

1. Coalesce repeated quit requests into one decision. If an export is active, offer **Wait for
   export** and **Cancel Quit**. Dismissing the prompt keeps the app open. The export continues
   on either choice, including when the user still needs to answer the native replacement prompt.
2. On Wait, keep the current export dialog available until that run reaches a terminal outcome.
   Show the existing success result and warnings, the error and its already-written paths, or
   the fact that replacement was cancelled. Let the user choose **Continue Quitting** or
   **Keep App Open** after reading the outcome. If output publication fails after the opening
   file was written, preserve the reported path from
   [`ExportError.written`](../../src-tauri/src/export_audio.rs). Keep the outcome stable while
   this decision is pending.
3. Before exiting, run the existing unsaved guard. Cancelling the prompt or failing to save
   keeps the app open and leaves the export outcome available. When there is no active export,
   go directly to the unsaved guard.
4. Track the pending request on the native side and accept completion only from the main
   webview for that request. Account for an export submitted just before Quit and the gap
   between the native worker releasing its permit and the frontend receiving its result.
   Coordinate the final native exit with
   [`ExportState::reserve`](../../src-tauri/src/export_audio.rs) so a new export cannot start
   between approval and exit. Keep one prompt during repeated requests and release the pending
   decision on cancellation or webview detachment. With an existing window and no ready
   listener, cancel Quit safely so it can be retried once the webview is ready.

## Dependencies

Task 95 is merged and provides native export, output publication, the result UI, and this
feature's starting contract. The app already has a macOS Objective-C bridge in
[`snapshot.rs`](../../src-tauri/src/snapshot.rs) and depends on `objc2` and `objc2-app-kit`.
Confirm the app delegate registration point, lifetime, and method signature against the locked
Tao version in an early macOS smoke test.

## Risks and validation

- Validate real Quit from the app menu, Cmd-Q, and Dock Quit on macOS. Check that the delegate
  callback receives each route, Tauri still performs normal shutdown, and Quit works during
  a native replacement prompt or is handled once that prompt clears.
- Cover Cancel Quit, Wait through success, a partial-write error, and replacement cancellation.
  Test a repeated Quit, an export that finishes as Quit arrives, and a late start around exit
  approval. An IPC error or stale reply must leave the app able to handle the next Quit.
- Exercise clean Quit and dirty-song Save, Discard, Cancel, and save failure. Re-run task 95's
  main-window close and export tests. Extend the existing frontend fake API tests and Rust
  export tests for new logic. Run the repository's Rust and frontend checks.

## Refinement decisions

- Cancel Quit keeps the export running. Keep that action available while waiting.
- Require an explicit click after the export finishes even when it succeeds, so
  the user has time to read the result before the app exits.
- A native destination picker can finish or be dismissed before a new Quit attempt. It does
  not start an export, so it needs no wait decision or result acknowledgement.
- Use the main webview's listener registration and page-load reset to scope request IDs to the
  live handler. The final exit takes the same native permit as export start and holds it
  until process shutdown.

Recommended complexity is **medium**. Keep singletonization and a light loss check. The proposal
fits in one document, so skip fracture.
