# Formatter and preference components

## Goal

Provide a pure formatter and an opt-in preference with backward-compatible storage.

## Prerequisites

Read the proposal and implementation contract.

## Implementation

The formatter owns `src/lib/editor/format-song.ts` and its dedicated test. The preference
component owns `src/lib/prefs/`, `tests/prefs.test.ts`, and the project's Rust preferences
module. The preference API provides `formatOnSave()` and `setFormatOnSave(on)`.

## Validation

Run the formatter and preference tests. Check timing parity, exclusion rules, idempotence,
old persisted preferences, delayed hydration, and alternating toggles.

## Commit message

`feat(editor): add Strudel pattern formatter` and
`feat(prefs): persist format-on-save setting`.
