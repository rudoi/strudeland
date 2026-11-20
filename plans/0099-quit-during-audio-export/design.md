# Quit and export contract

## Native lifecycle

- Keep the existing AppKit Quit action and intercept its delegate callback. Add the missing
  termination selector to the currently installed Tao delegate class on the main thread after
  checking the class identity and checking that no implementation already exists.
- Return cancel to AppKit immediately when the app queues a decision for the webview. The quit
  state tracks the main-window request channel, the pending request ID, and the final export
  permit. Page navigation removes the channel and invalidates the pending request.
- Only the main webview can register or answer. Accept a single matching reply. Cancel or
  delivery error clears the pending request, and stale replies have no effect. A main window
  without a ready listener cancels Quit. Without a main window, proceed only if the export
  permit is available.
- A positive reply acquires and retains the export permit before triggering the existing Tauri
  exit and cleanup. A busy permit rejects approval and allows a fresh Quit after export.
  Never hold the quit-state lock while sending a channel message or requesting Tauri exit.

## Webview behavior

- Use the current dialog controller as the source of truth for the export phase and outcome.
  A submitted export counts as active before native IPC returns. Show Wait for Export and
  Cancel Quit in the footer. Wait keeps the progress and Cancel Quit available.
- On completion, reuse the result or failure display and expose Continue Quitting and Keep App
  Open. A cancelled replacement gets its own terminal status. Retain the result until the
  quit request has settled and prevent a retry or a settings change during that time.
- For a clean idle flow or explicit Continue Quitting, call the existing unsaved guard.
  Keep the window open if consent or a save fails. A busy destination picker refuses the quit
  attempt safely. A pending application Quit also prevents independent window closure.
- Test event ordering and outcomes with the existing fakes and a native smoke test. The locked
  Tao delegate and AppKit lifecycle are part of the smoke test.
