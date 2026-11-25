# 003: the song header module

## Goal

Rename `src/lib/project/meta.ts` to `src/lib/project/song-header.ts` with `git mv`. It absorbs
`evaluate-options.ts` and exposes `readSongHeader(code)`. Every caller moves to it. See the
proposal's sections "`song-header.ts`", "Names (singletonized)" and "Callers".

## Prerequisites

002 is done.

## Implementation

- **`song-header.ts` public interface:**
  - `readSongHeader`, `headerRanges` and `LINE_BREAK`;
  - the types `Meta`, `MetaValue`, `MetaError` (with the `located` and `reported` getters) and
    `SongHeader`.
- **No longer exported:** `parseMeta`, `staticStructure`, `evaluateOptions` and `EMPTY_STRUCTURE`.
  Delete `evaluate-options.ts`.
- **Callers:** move every row of the proposal's callers table, keeping each caller's text
  byte-identical:
  - `session.ts`: `derive` uses `error.located`; `evaluateBuffer` keeps `meta: ${metaError}`, which
    equals `error.reported`.
  - `job-plan.ts`: uses `error.reported`. `EvaluatedSong.meta` stays.
  - `corpus.ts`: keeps `meta: ${error.message}` with no position, and drops its `buildStructure`
    catch.
  - `format-song.ts`, `format-script.ts`, `meta-write.ts` and `scripts/bench/scheduler.ts`.
- **`AGENTS.md`:** update the layout line that names `meta.ts` to `song-header.ts` and
  `readSongHeader`.

## Tests

- `tests/meta.test.ts` → `tests/song-header.test.ts`. One table of header texts, each row asserting
  the full result: `ok`, structure (or its key fields), options, title, and the exact error text and
  position, in all three forms. Fold in the cases from `tests/evaluate-options.test.ts`, then delete
  that file.
- Move these to `readSongHeader`: `tests/meta-write.test.ts`, `render-corpus.test.ts`,
  `render-job-plan.test.ts` and `runtime-backends.test.ts`.
- **Regeneration.** Run `bun run host:build`. This stage edits `job-plan.ts`, so also run
  `bun run engine:refs --write`; the jobs must not change. Check that the refs diff touches only
  provenance hashes.

## Validation

- `bun test`
- `bun run check`
- `bun run lint`
- `cargo test -p strudeland-engine` (the refs staleness test)

## Commit message

`refactor(project): read a song header through one module`
