# Implement loop export presets

Branch `llm/andrew/strudeland/0015-loop-export-modes`. Follow the [proposal](proposal.md), [resolved contract](design.md) and [comparison](loss-check.md). Love in the Tower's intro-once and loop-forever playback is the first acceptance case. Use the current worktree and configured Cargo target. The delivery PR is [PR #4](https://github.com/rudoi/strudeland/pull/4). Desktop export is tracked in task 95.

| Order | Stage | Test evidence | Commit scope |
| --- | --- | --- | --- |
| 1 | Planning | Host tests and standard host parity | Host protocol and scheduling |
| 2 | Finishing | Renderer, codec and decoded playback tests | PCM and metadata |
| 3 | CLI | CLI tests and actual export smoke | Flags, paths and output |
| 4 | Testing and staging | Workspace gates, quality review and guide updates | Fixes and documentation |
