# Export dialog

## Goal and prerequisites

Implement a desktop modal with the [command contract](contract.md) and existing app theme.

## Implementation

Place mode names and descriptions together, use a compact source header and put range and format
next to the destination and its resolved file list. Use a vertically scrollable modal if the
window is short. Keep the main action and state feedback easy to find. Wire the App menu event,
capture the active source, control focus and rival app shortcuts, and prompt before replacement.
Offer clip as a manual choice even when the static structure is missing. Treat static metadata as a
hint. The planner decides whether a selected loop or range is valid.

## Validation

Exercise choices and progress state, stale responses and error recovery with focused Bun tests.
Run frontend check, build, tests and lint on integration.

Commit message `feat(export): add the audio export dialog`.
