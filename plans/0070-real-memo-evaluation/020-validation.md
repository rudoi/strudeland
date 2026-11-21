# Validation and review

## Goal

Verify note accuracy, timing and playback in local and synthetic cases.

## Prerequisites

Both implementation changes and focused tests are ready.

## Implementation

Rebase on the fetched base. Compare the local reference and unchanged commands
using both default and fitted phase. Render or query the generated notation
with Strudel. Run the full Rust workflow gates, synthetic evaluation, and release
performance budget.

## Validation

Record actual changes in matches, false attacks, and missed notes. Pass the
synthetic pins and performance budget, inspect any differences by tier, and
complete the simplify, high-thoroughness review, and documentation gates
before opening the PR.

## Commit message

`docs(transcribe): record accuracy and playback validation`
