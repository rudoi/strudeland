---
name: simplify
description: Review and simplify recently changed code for reuse, clarity and efficiency while preserving behavior. Use for a simplify pass or the Strudeland workflow pre-push quality pass.
---

# Simplify changed code

Invoke with `$simplify` and an optional base, path or scope. Apply this reuse, clarity and efficiency
pass before the workflow's correctness review.

Establish scope before editing: honor an explicit target; in workflow use the full task diff against
the workflow’s selected main base; otherwise inspect staged, unstaged and untracked changes, or the current branch diff
when the working tree is clean. Resolve the merge base and inspect full changed files, relevant callers,
tests and `AGENTS.md`. Do not silently fall back to reviewing only the last commit. If a requested base
cannot be resolved, report it and resolve the target before claiming full coverage.

Review with three lenses:

- Reuse: find existing helpers/types that fit the change; consolidate actual duplicate logic.
- Quality: remove unnecessary indirection, redundant state, tangled control flow and misleading names
  where doing so makes the changed behavior easier to follow.
- Efficiency: examine hot-path allocations, repeated work and avoidable queries. Respect deterministic
  audio, audio-thread allocation rules and repo budgets. Measure when an efficiency claim needs evidence.

Apply concrete, behavior-preserving improvements within scope. Avoid speculative abstractions, unrelated
cleanup, public API churn and weakened tests. Preserve user changes. If the review reveals a behavioral
bug, distinguish its fix from simplification and cover it with an appropriate regression test.

For a substantial diff and when delegation is permitted, delegate read-only passes by lens or independent subsystem using the
[agent routing guide](../workflow/references/agents.md). Use Sol at medium effort for routine
analysis and Astra at high effort for subtle hot paths or cross-system contracts. Give agents the
same base and HEAD and state which local files belong to the task. Each suggestion needs the
current path, the existing code to reuse or the concrete cost to remove, and any behavior to verify.
Consolidate suggestions and apply verified edits in the parent session. Cover all three lenses directly when the diff is small or delegation is unavailable.

Run affected checks after edits; for meaningful Rust changes use focused tests and relevant formatting/
lint checks, with full delivery gates left to the workflow. Report scope, changes, validation and any
remaining concern. A no-change result is valid. Do not commit, push or open a PR unless the surrounding
workflow or user authorizes it.
