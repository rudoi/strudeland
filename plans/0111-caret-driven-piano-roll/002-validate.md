# Validate the complete interaction

## Goal

Reach a reviewable head with exact event semantics and verified desktop behavior.

## Prerequisites

Finish the integrated edit path in [implementation](001-build.md), including regenerated host
artifacts. Rebase on latest origin/main with a clean task-owned tree.

## Validation

- Run Bun build, test, check and lint. Run Rust workspace test, Clippy and rustfmt for changes to
  the host contract and bundled runtime.
- Compare host and render-page jobs for mixed meter and a new event-edit fixture. Query the local
  Luoyuan acceptance source and a temporary edited copy. Check whole spans, unaffected events,
  glide dependencies, and pitch and rhythm in the rendered arrangement.
- Smoke the normal dock entry and first drag for a caret in mini notation, a legacy grid,
  a transformed reference and an arranged event. Observe source movement, roll focus, pulse,
  undo, repeat drags, reduced motion, collapse and stale-state cancellation. Capture a screenshot
  and check errors. Record any unavailable native or accessibility evidence accurately.
- Resolve real validation failures and record results. Proceed to staging only when implementation
  satisfies the binding requirements, then open the PR and stop on review gate entry.

## Commit message

`docs(editor): record caret piano roll verification`
