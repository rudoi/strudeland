# 004: a typed error kind for a failed evaluate

## Goal

`session.evaluateBuffer` returns `SessionEvalOutcome`, which adds `errorKind` to the wire fields.
`backing.ts` reads `errorKind`, not the `meta: ` prefix. The control wire is unchanged. See the
proposal's sections "The error kind" and "Names (singletonized)".

## Prerequisites

003 is done.

## Implementation

- **`session.ts`:**
  - export `EvalErrorKind` and `SessionEvalOutcome`;
  - `evaluateBuffer` sets `errorKind` with the same precedence as its error text: `meta`, then
    `instruments`, then `eval`; `null` exactly when `ok`.
  - The interface's `evaluateBuffer` type changes to `SessionEvalOutcome`.
- **`recording/backing.ts`:** the deps type takes `SessionEvalOutcome`. `meta` when
  `errorKind === 'meta'`, else `eval`. Delete `META_PREFIX`.
- **`control/handler.ts`:** `evaluateGuarded` maps the session result to the wire `EvalOutcome`
  field by field: `ok`, `song`, `title`, `started`, `audio`, `error`.
- **Test fakes:** update any fake session in `tests/helpers/` that returns `evaluateBuffer`
  results.

## Tests

- `tests/session.test.ts`: `errorKind` for a meta error, an instrument pin error, a runtime eval
  error and success. The existing text assertions stay.
- `tests/recording-backing.test.ts`: a meta failure gives `meta`; an instrument-pin failure and an
  eval failure give `eval`.
- `tests/control-handler.test.ts`: the `evaluate` result and `code set`'s `evaluated` have exactly
  the wire keys (`toEqual` or `Object.keys`), with no `errorKind`.

## Validation

- `bun test`
- `bun run check`
- `bun run lint`

## Commit message

`refactor(session): report why an evaluate failed as a typed kind`
