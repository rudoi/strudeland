# Export decisions and unsaved guard

## Goal

Present the wait, cancellation and outcome choices in the export dialog and preserve unsaved
songs.

## Prerequisites

Stage 010 and the webview behavior in the design.

## Implementation

Connect the shell to native Quit requests and the existing unsaved guard. Extend the export
controller and footer with one pending quit decision and the terminal outcome. Keep frontend
state locked until the request settles.

## Validation

Use the export fakes for the decision and result order, partial publication, replacement
cancellation, pending request deduplication and re-enabling controls. Exercise the shell's
unsaved and close guards. Complete native smoke before Staging.

## Commit message

`feat(export): show export outcome before quitting`
