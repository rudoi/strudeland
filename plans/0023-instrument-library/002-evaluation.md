# Compile and bind instrument definitions

## Goal

Allow songs to apply pinned instruments and kits through Strudel on desktop and headless.

## Prerequisites

Use the shared schema and [evaluation contract](evaluation.md).

## Implementation

Build lexical namespace factories in the shared adapter. Register the fixed transform at
prebake and pass snapshots via the queued runtime evaluation. Integrate TS host and render
page inputs.

## Validation

Compare hap values, voice labels and locations. Test failures, shadowing and old snapshots,
and confirm ordinary evaluations still work.

## Commit message

`feat(instruments): bind pinned presets during evaluation`
