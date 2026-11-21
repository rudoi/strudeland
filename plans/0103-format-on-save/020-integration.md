# Save integration

## Goal

Route save entry points through a shared formatting and snapshot contract, with an editor toggle.

## Prerequisites

The formatter and preference APIs from stage 010. Tests may be prepared while that stage
is in progress.

## Implementation

Apply the pure edits through the session save and history paths. Supply the preference from
app composition and expose it beside the existing editor controls. Keep native writes and
watcher handling on the resulting snapshot.

## Validation

Exercise normal and stashed saves, undo, failed writes, edits during a save and exact evaluation
text. Cover all affected save entry points by ensuring they use the common write. Run the
repository build, typecheck, test and lint commands. Launch the desktop app and inspect the
control and a formatted example with playback if the environment permits it.

## Commit message

`feat(editor): format mini-notation on save`.
