# Store and validate definitions

## Goal

Allow an editable global library, self-contained portable project pins and explicit updates.

## Prerequisites

Use the [schema and store contract](storage.md).

## Implementation

Add the versioned definition and store in strudeland-project. Add one canonical shared table
of core scalar controls. Keep dependency lock updates compatible with the workspace.

## Validation

Exercise bad schemas and references, legacy projects, unchanged pins after global edits,
stale expected fingerprints, and unsafe paths. Run focused project crate tests.

## Commit message

`feat(instruments): store reusable definitions and project pins`
