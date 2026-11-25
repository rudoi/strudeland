# 002: the song playback module

## Goal

Add `src/lib/project/song-playback.ts` exactly as the proposal specifies. That covers its store,
the verbs and their contracts, name resolution, and how it follows the runtime.

The session constructs it, exposes it as `session.playback`, and calls its verbs in place of
every `runtime.transport` call. `SessionState.evaluated` moves into the module's store.

## Prerequisites

001 is done.

## Implementation

- **The module.** Build it with:
  - `runtime`, called through the object at call time;
  - `getHeader(): { active, structure, metaError }`;
  - `currentGen()`;
  - `titleFrom`, exported from where it lives now, or moved to `song-header.ts`.
- **In `session.ts`:**
  - `evaluateSong` keeps its header and instrument gating and calls `refused()` where it reset
    before. Otherwise it calls `playback.evaluate(path, text, opts, generation)`. That must stay
    synchronous up to `runtime.evaluate`.
  - `install` calls `projectChanged()`.
  - `activeTransport` becomes `activeChanged()`.
  - Rename calls `renamed(from, to)` inside `whileMoving`.
  - `deleteTask` calls `activeChanged()`, then `removed(path)`, and keeps `stopped`.
  - `pianoRollMatchesLivePins` reads `playback.instruments()`.
  - `SessionState.evaluated` and the internal `evaluated` variable are removed.
- **Callers outside the session** keep compiling in this stage. Where one read
  `$ss.evaluated` or `get(session.store).evaluated`, point it at `session.playback`'s store with
  the same values. Stage 003 makes the real caller moves.
- **`AGENTS.md`:** add `song-playback.ts` to the `src/lib/project/` list.

## Tests

- **New: `tests/song-playback.test.ts`.** Every module case in the proposal's Tests section,
  except the caller-level ones (`playFrom`, backing, handler), which come in 003.
- **`tests/session.test.ts`:**
  - Move the `evaluated` assertions to `session.playback`.
  - Move the `transport reset` block and the delete-reset tests into the module tests as
    outcomes.
  - Remove `resets()` from the stub once nothing uses it.
- **`tests/export-dialog.test.ts`:** drop `evaluated` from its `SessionState`.

## Validation

- `bun test`
- `bun run check`
- `bun run lint`

## Commit message

`refactor(project): give the playing song and its transport one owner`
