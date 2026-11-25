# 001: extract the transport model

## Goal

The transport glue moves out of `runtime.ts` into an exported factory,
`createTransportModel({ started, structure, boundary, now })`, in `src/lib/strudel/transport.ts`.
The glue is:

- `setSegments` (around `:421-456`);
- `change`, `startFrom`, `loopSection`, `loopRange`, `playEnding` and `reset` (around `:835-890`);
- the rewind when `started` goes from true to false (around `:639-643`).

The runtime builds its transport with this factory. Its behaviour does not change.

`tests/helpers/stub-runtime.ts` builds its transport with the same factory. The test controls
`boundary` and `now`, and the stub's `started` store feeds the rewind.

See the proposal's section "Transport model (`transport.ts`)".

## Prerequisites

None.

## Implementation

- **Keep the `Transport` interface and `TransportState` exactly as they are.** The factory
  returns them.
- **`boundary`** covers what `change()` needs while playing: the scheduler's `lastEnd`, `toCycle`,
  `pruneAt` and mixed-meter timing. The runtime passes closures over its scheduler.
- **The stub:**
  - It keeps its recorded call list (`transportCalls`) for now, so the existing tests still pass
    in this stage. Stage 002 and 003 remove it, along with `resets()` and `setTransport`, once
    their tests move to outcomes.
  - If keeping `setTransport` working on top of the model is awkward, keep it in this stage as a
    test-only override.

## Tests

- **`tests/runtime.test.ts` and `tests/runtime-backends.test.ts`** must pass unchanged. They are
  the conformance suite.
- **New: `tests/transport-model.test.ts`.** Direct tests of the factory with a fake `boundary`
  and `now`:
  - start-from and loop while stopped;
  - an append at the next bar while playing;
  - the rewind on stop;
  - `reset`;
  - the release view.

## Validation

- `bun run host:build`
- `bun test`
- `bun run check`
- `bun run lint`

## Commit message

`refactor(runtime): share the transport model with the test runtime`
