# 003: the three approved visible changes

## Goal

1. **Cmd/Ctrl-. stops like the toolbar.** `app.ts` wires `onStop: () => void arbiter.stop()`.
2. **The Timeline's section items are disabled when `!canPlayFrom`.** That covers start here,
   loop this section and each "through …". The help precedence is the proposal's: the resolve
   failure, then "Stop the take first." or "Stop the memo first." (by `stopTarget`), then the
   existing branches. `play ending` and `stop looping` are unchanged.
3. **Cmd-Enter flashes only when the evaluate runs.** `app.ts` wires
   `onEvaluate: () => arbiter.play()`.

## Tests

- **`tests/app.test.ts`:**
  - during a take with song backing, Cmd-. ends the take, and the runtime keeps playing until the
    take stops it;
  - during a memo with song backing, Cmd-. stops both;
  - Cmd-Enter during a take doesn't flash and doesn't evaluate.
- **`tests/play-arbiter.test.ts`:** rows for `canPlayFrom` across every state in the Timeline
  spec. The Timeline component reads that field; there is no component test.

## Validation

- `bun test`
- `bun run check`
- `bun run lint`
- `bun run build`

## Commit

`feat(app): stop, flash and arm sections by the same play rules everywhere`
