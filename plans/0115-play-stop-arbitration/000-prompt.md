# 0115 implementation entry point

- **Branch:** `llm/andrew/strudeland/0115-play-stop-arbitration`.
- **Proposal:** [../0115-play-stop-arbitration.md](../0115-play-stop-arbitration.md). It is the
  contract.
- **Evidence:** [reverify.md](reverify.md).

## Stages

The stages run in order.

1. [001-arbiter.md](001-arbiter.md): the module and its table tests. No callers change.
2. [002-callers.md](002-callers.md): every caller moves onto it. No visible change.
3. [003-approved-changes.md](003-approved-changes.md): the three approved visible changes.

## Rules for every stage

- **Behaviour is frozen in stages 001 and 002.** Stage 003 makes only the three approved changes.
  The control wire and CLI output never change.
- **Replace tests, don't layer them.** New logic ships with tests in the same commit.
- **Gesture.** Nothing awaits before `runtime.evaluate`, `runtime.start` or `resumeAudio`.
  `App.svelte` stays untouched.
- **Host bundle.** Check `crates/strudeland-host/js/host.inputs.json` for every edited file, and
  run `bun run host:build` if one is listed. None is expected.
- **Test cadence.** Targeted `bun test tests/<file>` while working. When the stage is done, run
  `bun test`, `bun run check` and `bun run lint` once, logging output and exit codes.
- **Commits are plain Conventional Commits.** No task or plan identifiers. End each with the
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` trailer. Run `bun run format`
  first.
- **No scratch worktrees,** and no bare `git stash`.
