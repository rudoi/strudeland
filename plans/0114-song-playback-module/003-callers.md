# 003: move the callers onto the module

## Goal

Every reader of "what is playing" or "what is armed" uses `session.playback`. Nothing outside the
module touches `runtime.transport`. `matchingSectionRange` is gone. Decision 7 lands.

## Prerequisites

002 is done.

## Implementation

Follow the proposal's Callers table and Wiring section:

- **`App.svelte`:** `playingPath` from `playing`.
- **`Timeline.svelte`:**
  - read `playing`, `transport` and `canArm`;
  - arm through `app.playFrom`;
  - `stopLooping` and `playEnding` through the module;
  - keep the playhead, `canPlayEnding` (fed `transport.view`), `playable` and the `head`
    translation.
- **`PianoRoll.svelte`:** `matchesLive` and the playhead gate.
- **`control/handler.ts`:**
  - `state.playing`, `state.started` and `state.transport` come from the module, with the wire
    shape unchanged;
  - `events` reads `evaluated`;
  - `shouldAutoEval` gets `playback.evaluated?.path`.
- **`recording/controller.ts`:** read the armed and loop names before stopping.
- **`recording/backing.ts`:** the section start becomes `playback.start(...)`. `BackingStarter`
  is unchanged.
- **`app.ts`:** add `playFrom(choice)` with the three guard paths, and pass `session.playback` to
  the handler, backing and controller deps.
- **Cleanup:**
  - delete `matchingSectionRange` from `project/timeline.ts`;
  - remove the stub's `transportCalls` and `setTransport` once unused.

## Tests

- **`tests/control-handler.test.ts`:**
  - transport tests through module verbs;
  - decision 7's three cases pinned;
  - the null-structure case at `:1091-1095` rewritten;
  - the out-of-range-index case removed.
- **`tests/recording-controller.test.ts`** (`:592-630`) and **`tests/recording-backing.test.ts`**
  (`:210-262`): outcomes, not call lists.
- **`tests/timeline.test.ts`:** the `matchingSectionRange` tests have moved; delete them here.
- **`tests/app.test.ts`:**
  - the new wiring;
  - `playFrom`'s three guard paths: a take arms only; stopped arms then evaluates; playing arms
    only.
- **`tests/song-playback.test.ts`:** add backing's `start` (disarm on failure, no await before
  `runtime.start`).

## Validation

- `bun test`
- `bun run check`
- `bun run lint`
- `bun run build`

## Commit message

`refactor(project): read the playing song and armed section from one module`
