# 002: one owner for the cross-field rules

## Goal

`buildStructure` becomes the only place the cross-field rules live. The reader of the parsed
header translates a failing field into its source node. See the proposal's sections "`structure.ts`"
and "Inside".

## Prerequisites

001 is done.

## Implementation

`src/lib/strudel/structure.ts`:

- Make `StructureError` take a required `field`: `bpm`, `meter`, `sections`, `loop`,
  `meterChanges` or `loops`.
- `check` takes the field. Every caller passes it, including `withLoops` (`loops`).
- The thrown `'meta.loop cannot be used with section loop: true'` passes `loop`.
- Messages are unchanged.

`src/lib/project/meta.ts` (still under that name in this stage), `readHeader`:

- Delete the cross-field block (`:262-288`).
- After `toObject` and the per-field checks, call `buildStructure(meta)` once and resolve the first
  error in the proposal's order:
  1. A `loop` or `meterChanges` error is reported at that key's last occurrence node.
  2. Otherwise, range-check the earlier duplicate `loops` occurrences in source order with
     `loopsProblem(value, true)`, and report the first failure at its node.
  3. Otherwise, a `loops` error is reported at the last `loops` occurrence.
- Keep the last occurrence node per key, as `layoutFields` does today.
- `readHeader` returns the built structure as well, so stage 003 can reuse it. `parseMeta` and
  `headerRanges` keep their current interface in this stage.
- Do not import `meta.ts` from `structure.ts`. `structure.ts` stays free of Strudel, Tauri, DOM and
  `project/` imports.

## Tests

- `tests/structure.test.ts`:
  - assert `field` on each cross-field error;
  - update the one-argument `new StructureError(msg)` calls (around `:386-390`).
- `tests/meta.test.ts` (it moves in 003):
  - add first-error rows with combined faults, including
    `{ sections: [{ name: 'a', bars: 1 }], loop: { from: 'x', through: 'a' }, loops: 0, loops: 2 }`
    (the loop error wins, at the loop node) and `{ loops: 0, loops: 2 }` (the range error wins, at
    the first `loops` node);
  - add a row where the earlier `loops` is invalid and `buildStructure` succeeds:
    `{ sections: [{ name: 'a', bars: 1, loop: true }], loops: 0, loops: 2 }`.
- **A pairing test.** For each cross-field rule, assert that the header text through `parseMeta`
  and the evaluated object through `buildStructure` give the same message.
- **Behaviour first.** Record the current results of the new rows *before* changing `readHeader`,
  so the tests prove the order is unchanged.

`structure.ts` and `meta.ts` are bundled into the host, so run `bun run host:build`.

## Validation

- `bun test`
- `bun run check`
- `bun run lint`

## Commit message

`refactor(project): let buildStructure own the cross-field header rules`
