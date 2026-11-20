# Proposal coverage

| Requirement | Design coverage |
| --- | --- |
| Quit menu, Cmd-Q, and Dock Quit | AppKit delegate callback with native smoke coverage |
| Deliberate wait or cancellation | Export footer offers both choices and keeps cancellation available during the wait |
| Visible result or partial publication | The result and error views survive until the user chooses whether to continue quitting |
| Unsaved changes | Continue through the existing save/discard/cancel guard and refuse exit when a save fails |
| Main-window close and export safety | Keep the existing close guard and native output publication. Block window closure while the app quit decision is pending |
| Late starts, duplicate requests, and detached listeners | Frontend activity, native request IDs, and export permit ownership cover the separate races |
| Existing shutdown | Approved requests exit through Tauri so the normal cleanup runs |

The refinement made the choice inline and made cancellation available while waiting. One proposal
and one contract cover the feature. No requested behavior was dropped.
