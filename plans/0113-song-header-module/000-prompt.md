# 0113 implementation entry point

- **Branch:** `llm/andrew/strudeland/0113-song-header-module`.
- **Proposal:** [../0113-song-header-module.md](../0113-song-header-module.md). Read it first; it
  is the contract.
- **Evidence:** [reverify.md](reverify.md).

## Stages

The stages run in order. Each one depends on the one before.

1. [001-rename-expected-structure.md](001-rename-expected-structure.md)
2. [002-cross-field-rules.md](002-cross-field-rules.md)
3. [003-song-header-module.md](003-song-header-module.md)
4. [004-eval-error-kind.md](004-eval-error-kind.md)

## Rules for every stage

- **Behaviour is frozen.** Every user-visible message, error position, control-protocol text and
  engine job byte stays identical. A test that needs its expected text changed is a bug in the
  stage, not in the test. The only exception is the moved test files the proposal names.
- **Replace tests, don't layer them.** New logic ships with tests in the same commit.
- **Keep `bun test` green in every commit.** If a stage edits a file the headless host bundles
  (listed in `crates/strudeland-host/js/host.inputs.json`), run `bun run host:build` and commit the
  regenerated files with it. If it edits `src/lib/render/job-plan.ts`, also run
  `bun run engine:refs --write` (needs Chromium) and commit the refs.
- **Test cadence.** While working, run only the targeted tests (`bun test tests/<file>`). When the
  stage is done, run its validation list once and log the output and the exit codes.
- **Commits are plain Conventional Commits.** No task or plan identifiers. End each one with the
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` trailer.
- **Run `bun run format` before committing.**
- **No scratch worktrees.** Work in this checkout. Never use `git stash` without a unique tag.
