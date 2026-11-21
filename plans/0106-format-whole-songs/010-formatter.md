# Implement and test whole-song formatting

## Goal

Save a song with canonical layout for chains and common arrangements while keeping the existing
rhythmic mini layout and save behavior.

## Prerequisites

Read the [contract](design.md), current `format-song.ts`, and session save tests.

## Implementation

Implement a syntax- and token-guided JavaScript pass, then coordinate it with the existing mini
pass through localized edits. Update the tooltip and extend pure formatter and session tests.
The session API remains the sole save integration.

## Validation

Run focused formatter and session tests. Check exact text, token spelling, repeat saves, mixed
mini and JavaScript, and session mapping. Measure locally against the checked-in song corpus.
The Testing stage runs the full frontend commands and a desktop smoke. The Staging stage runs
simplify, high code review, and documentation checks.

## Commit message

`feat(editor): format whole Strudel songs on save`
