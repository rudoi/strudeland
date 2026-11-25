# 002: move the callers onto the arbiter

## Goal

Every UI and control entry point uses the arbiter, as the proposal's Callers table and Editor
hooks section specify. There is no visible change.

- **Cmd-. keeps today's behaviour in this stage.** `onStop` is wired in stage 003.
- **Cmd-Enter keeps flashing in this stage.** The return value is wired in stage 003.
- **The Timeline's disabling is stage 003.**

## Implementation

- **`app.ts`:** builds the arbiter, moves `takeActive`, `memoActive`, `playRequests` and the
  `stopAuditions` subscriptions into it, and calls `arbiter.destroy()` in `app.destroy` before
  the memo and recording teardown. `evaluateDoc` and `playFrom` delegate to it. Audition gating
  calls `auditionsAvailable()`. `beforeProjectChange` is unchanged.
- **`editor/setup.ts`:** the hook types gain `onEvaluate?: () => boolean | void` and
  `onStop?: () => void`, implemented with the flash and fallback contract. The app doesn't use the
  new behaviour until 003.
- **`Toolbar.svelte`, `RecordControls.svelte`, `InstrumentBrowser.svelte` and
  `PianoRoll.svelte`:** read and call the arbiter. The copy text is unchanged.
- **`Timeline.svelte`:** items call `arbiter.playFrom`, and its `taking` reads the arbiter. No
  disabling yet.
- **`control/handler.ts`:**
  - it takes `Pick<PlayArbiter, 'evaluate' | 'stop'>`, defaulting to an arbiter built from
    `{ session, runtime }`;
  - `stop` awaits `done`;
  - the wire stays byte-identical;
  - `ControlPlayback` and `NO_PLAYBACK` are removed.
- **`AGENTS.md`:** lists `play-arbiter.ts`.

## Tests

- **`tests/control-handler.test.ts`:** the memo-and-take block becomes one wiring test with a
  fake arbiter. `:588`, `:605` and `:728` stay.
- **`tests/app.test.ts`:** the playFrom and take tests move to the arbiter suite, and one
  integration test stays. Every monkey-patching test still passes.
- **`tests/setup.test.ts`:** the new hook contracts: a `false` return means no flash, and
  `onStop` is called when given. The fallback tests are unchanged.

## Validation

- `bun test`
- `bun run check`
- `bun run lint`
- `bun run build`

## Commit

`refactor(app): route play and stop through the play arbiter`
