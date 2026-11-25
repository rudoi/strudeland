# 001: rename `expectedArrangement` to `expectedStructure`

## Goal

`EvaluateOptions.expectedArrangement` (`src/lib/strudel/runtime.ts:65`) holds the song's structure.
In `GLOSSARY.md`, "arrangement" means the written order of patterns, so rename the field to
`expectedStructure`.

## Prerequisites

None.

## Implementation

- `src/lib/strudel/runtime.ts`: the field (`:65`) and its two reads (`:485`, `:664`). Update the doc
  comment if it says "arrangement".
- `src/lib/project/evaluate-options.ts`: the object key.
- Tests: `tests/session.test.ts` (around `:2760`) and `tests/runtime-backends.test.ts` (around
  `:498`); `tests/evaluate-options.test.ts` if it names the field.
- `grep -rn expectedArrangement src scripts tests` must come back empty, except the bundled
  `host.js` before it is rebuilt.
- `runtime.ts` and `evaluate-options.ts` are bundled into the host, so run `bun run host:build`.

## Validation

- `bun test`
- `bun run check`
- `bun run lint`

## Commit message

`refactor(runtime): call the expected arrangement the expected structure`
