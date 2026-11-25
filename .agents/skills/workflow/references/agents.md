# Subagent routing

Choose the lightest currently available model and reasoning effort that can own the assignment.
Honor any user override. Keep the parent session's model and settings as configured.

| Assignment | Model | Effort |
| --- | --- | --- |
| Locate files and symbols, inventory a diff, compare an explicit checklist | `gpt-6.2-sol` | `low` |
| Verify plan references, fracture documents, fill settled details, sequence work, implement routine wiring and tests | `gpt-6.2-sol` | `medium` |
| Trace a cross-crate design, resolve a nontrivial loss check, implement tricky Rust, Svelte or audio logic, diagnose a difficult failure | `gpt-6.2-astra` | `high` |
| Review correctness in `$code-review high`, or investigate a subtle contract or determinism failure | `gpt-6.2-astra` | `high` |
| Inspect screenshots or visual playback evidence where visual judgment dominates | `gpt-6.1-terra` | `medium` |

Use Sol at medium effort for an ambiguous routine assignment. Escalate when evidence points to
cross-system invariants, ambiguous failures, or a contract that the first pass could not resolve.
Handle simple status checks, Git reconciliation, staging and parking directly. High review
selects the review depth even when only inherited model settings are available.

On the collaboration tool that exposes `model`, `reasoning_effort` and `fork_turns`, pass the selected
model and effort explicitly with `fork_turns: "none"` when the tool permits overrides. If active
instructions require inheritance, use the inherited settings. Supply a self-contained assignment including
the exact worktree, base and HEAD or stage, goal, relevant user constraints and plan decisions,
paths to read, write ownership, validation and what to return. For an independent review, give the
raw target and contracts without seeding hypotheses. Use the live tool schema as the source of
supported settings. On surfaces with only configured agent roles or inherited settings, choose an
available role with the same capability profile, or delegate using the available defaults. If
delegation is unavailable, do the work in the parent session and disclose the limitation where it
affects the result. Report the model and effort only when the runtime accepted those settings.

Keep the following constraints in each assignment where they apply.

- Stay in the provided user-created worktree. No new worktrees or branch switches. The parent
  handles workflow state, commits, tracker updates and all remote mutations.
- Run read-only research and review concurrently when independent. Assign disjoint paths to
  concurrent editors only when the plan allows it. Run dependent stages in order.
- Coordinate Cargo checks with the parent. Only one Cargo command at a time in this worktree.
  Static reviewers can continue while the designated tester runs a command.

Ask for file and line evidence, test results and gaps, or the exact edits and checks performed,
depending on the assignment. The parent verifies findings and validates integrated edits before
recording completion.
