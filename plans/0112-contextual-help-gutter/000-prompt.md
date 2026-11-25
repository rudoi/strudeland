# Contextual help implementation

Task 112 on branch `llm/andrew/strudeland/0112-contextual-help-gutter`. Work in the current user-created worktree. The [proposal](proposal.md), [interaction](help-interaction.md), [workspace integration](workspace-settings.md), [coverage](coverage.md) and [loss check](loss-check.md) are retained design records.

Implement [help bindings](010-bindings.md), [workspace and Settings](020-settings.md), and [control coverage](030-coverage.md) with tests for new logic. The three stages share only the Help interface and can be edited concurrently once that interface is agreed. Integrate in that order. Keep the log and retire the numbered execution files at finalization. Run the full relevant frontend and native gates and desktop smoke tests. Stop at PR review for human approval.
