# 4. Integration and validation

## Goal

Verify full playback and export and prepare the changes for review.

## Prerequisites

Implementation in stages 1 through 3.

## Implementation

Rebuild the host bundle and regenerate changed references and vectors using the supported commands. Update documentation. Run repository testing, browser smoke, parity, and the quality passes in the workflow skill.

## Validation

Compare browser and host plans. Assert decoded repeat frames and both handoffs. Run frontend build, tests, typecheck, lint, and Rust workspace tests, clippy, and formatting. Record any test failure and the verified disposition in the implementation log.

## Commit message

`test(loops): verify mixed-meter playback and export`
