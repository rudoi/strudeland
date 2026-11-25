# 0114 implementation entry point

- **Branch:** `llm/andrew/strudeland/0114-song-playback-module`.
- **Proposal:** [proposal.md](proposal.md). It is the
  contract.
- **Evidence:** [reverify.md](reverify.md).

## Stages

The stages run in order. Each one depends on the one before.

1. [001-transport-model.md](001-transport-model.md)
2. [002-song-playback-module.md](002-song-playback-module.md)
3. [003-callers.md](003-callers.md)

## Rules for every stage

- **Behaviour is frozen, except decision 7.** Every user-visible message, control-protocol output
  and CLI output stays identical, apart from decision 7's three armed-name cases. A test that needs
  its expected output changed is a bug in the stage, with two exceptions: tests that assert call
  order or raw transport state, which the proposal replaces with outcome tests, and decision 7's
  case.
- **Replace tests, don't layer them.** New logic ships with tests in the same commit.
- **Keep `bun test` green in every commit.** `runtime.ts` and `transport.ts` are bundled into the
  headless host, so any commit touching them runs `bun run host:build` and commits the output.
  Neither is a planner file, so there are no engine refs to rewrite.
- **Test cadence.** While working, run only targeted tests (`bun test tests/<file>`). When the
  stage is done, run `bun test`, `bun run check` and `bun run lint` once, logging their output and
  exit codes.
- **Commits are plain Conventional Commits.** No task or plan identifiers. End each with the
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` trailer.
- **Run `bun run format` before committing.**
- **No scratch worktrees,** and no bare `git stash`.
