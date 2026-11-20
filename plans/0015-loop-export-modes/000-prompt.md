# Implement loop export presets

Branch `llm/andrew/strudeland/0015-loop-export-modes`. Follow the [proposal](proposal.md), [resolved contract](design.md) and [comparison](loss-check.md). Love in the Tower's intro-once and loop-forever playback is the first acceptance case. Use the current worktree and configured Cargo target. The user authorized automatic progress through PR creation. Stop at the review gate.

| Order | Stage | Test evidence | Commit scope |
| --- | --- | --- | --- |
| 1 | [Planning](010-planning.md) | Host tests and standard host parity | Host protocol and scheduling |
| 2 | [Finishing](020-finishing.md) | Renderer, codec and decoded playback tests | PCM and metadata |
| 3 | [CLI](030-cli.md) | CLI tests and actual export smoke | Flags, paths and output |
| 4 | Testing and staging | Workspace gates, quality review and guide updates | Fixes and documentation |

Keep this entry point, the proposal, resolved contract and comparison. Remove the three numbered execution files during finalization.
