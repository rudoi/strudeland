# Implement the source-backed piano roll

## Goal

Drag any shown pitched note, commit immediately to the editor, reveal the change and flash the
affected notes. Give the original notation the first opportunity to express the change.

## Prerequisites

Read [the proposal](proposal.md) and the three contracts from [the entry point](000-prompt.md).
Agree on shared types and scope before wiring together edits to the same song.

## Implementation

- Source resolution and edit routing own `source.ts` and source resolver tests. Support caret
  inference, verified row arrays and occurrence-sensitive routing. Preserve exact explicit selection.
- Grid codec and reusable constructor own new grid modules and tests. Coordinate runtime registration
  with the runtime task owner. Keep legacy recognition bound to the existing lexical analysis.
- Runtime and host own note identity, edit operation, production source planning, and shared
  registration. Rebuild the committed host artifacts after integrating the new runtime scope.
- Dock and editor own the grid component, session transaction, scroll effect, focus, and visual
  feedback. Reconcile prepared edits with current snapshots and host responses. Keep input focus
  and the played-note accent independent of edit animations.

Parent owns shared-file integration, commits, and gates. No concurrent Cargo commands. Coordinate
changes in `types.ts`, runtime registration, and inspection request/response contracts explicitly.

## Validation

Each contribution ships with focused tests. Cover genuine caret positions, authored grid event
parity, shadowing, alternate and layered notes, disjoint query intervals and selector collisions,
stale or changed document handling, keyboard and pointer interaction, focus, and history.

## Commit messages

- `feat(editor): resolve piano roll targets from the caret`
- `feat(editor): edit authored note grids in place`
- `feat(runtime): persist piano roll edits for evaluated notes`
- `feat(editor): reveal and highlight piano roll edits`
