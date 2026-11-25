# 001: the play arbiter

## Goal

Add `src/lib/play-arbiter.ts` exactly as the proposal specifies:

- the construction;
- the store, including `canPlayFrom`, `stopEnabled` and `auditionsAvailable()`;
- the verbs and their contracts: `play`, `playFrom`, `stop` with `done`, `evaluate`, `record`,
  `destroy`;
- the `stopAuditions` side effects when a take or an audible memo begins.

No caller changes in this stage. The module encodes today's rules, with `canPlayFrom` computed
but not yet used.

## Tests

`tests/play-arbiter.test.ts` holds the whole table from the proposal's Tests section, over the
stub runtime, the fake recording API and a small memo fake. It includes:

- the `wasPlaying` rows;
- stop cancelling an in-flight autostart;
- stop silencing an audition while idle;
- `auditionsAvailable` during a pending play;
- the kept quirks for #141 and #142, as explicit rows.

## Validation

- `bun test`
- `bun run check`
- `bun run lint`

## Commit

`feat(app): add a play arbiter for song, memo and take`
