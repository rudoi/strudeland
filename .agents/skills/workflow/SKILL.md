---
name: workflow
description: Run or resume Strudeland's staged delivery workflow from a task or feature through planning, implementation, validation, quality review, PR, and human-approved finalization.
---

# Strudeland workflow for Codex

This is the Codex counterpart of `.claude/commands/workflow.md`. Maintain this copy independently;
do not execute the Claude command or require its runtime. Invoke with `$workflow` and ordinary text
(e.g. `$workflow resume`, `$workflow plan task 42`, `$workflow stage this branch`). Arguments come
from the user's message, not a `$ARGUMENTS` variable. Merely editing this skill does not start a workflow.

## Codex execution

- Delegate independent research, implementation stages and review to native subagents where useful
  and permitted by active instructions.
  Select the model and effort for each assignment using [agent routing](references/agents.md).
  Keep stage gates, integration, Git publication and task tracking in the parent session.
- Read the sibling Codex skills directly at Staging. Use repository launch and capture commands for
  smoke tests. Operate only in the current user-created worktree, including all delegated work.
- Follow `AGENTS.md` and the task’s plan contracts. Read `CLAUDE.md` only for
  additional applicable repo conventions, resolving conflicts in favor of `AGENTS.md` and user intent.

## Sources, tasks, branches

Code establishes implementation state. Plans establish intended behavior. Read relevant sections of
`docs/build-plan.md` if present and the applicable plan under `plans/`; surface material disagreements
before changing the design. Use `keitora task` for tracking (try `~/.local/bin/keitora` if absent
from PATH). Inspect `--help` before unfamiliar mutations and use `--json` where supported. If unavailable, continue independent research and report
what tracking work remains; never fabricate task IDs or successful updates.

For a fresh workflow, inspect tasks and `keitora task list --mine`. Continue a matching task when the
user has requested it. Filter for active tasks because `--mine` can include completed assignments.
If another task is active here, resolve which to continue before creating or claiming a task. Create
a task for an untracked feature. Claim a selected open task before starting, whether existing
or newly created, and respect current ownership and dependency blockers. Work number is the task ID,
four-digit zero padded. Plans live at `plans/{NNNN}-{description}.md` and/or
`plans/{NNNN}-{description}/`. Start new branches at fetched `origin/main`, or local `main`
when no remote main exists, using the shared name
`llm/<git-user>/strudeland/{NNNN}-{description}` (first word of `git config user.name`, lowercased;
validate with `git check-ref-format --branch`). Preserve existing task branches, including `codex/...`
branches. Check other worktree branch ownership before switching and preserve unrelated edits.

File deferred work immediately with `keitora task new "title" -d "..." -t <tags>`. If it depends on the
current task, run `keitora task block <new-id> <current-id>`. Reference IDs in plan docs, never in code or commit messages.
Use Conventional Commits and actual contributor identities. Treat Claude-specific trailers in older
stage prompts as historical. Commit only task-owned files with explicit paths.

## State and stage gates

Keep the live `WORKFLOW_STATE.json` at the repo root (already gitignored). Read it on every invocation.
Preserve unknown version-1 fields and the recorded branch, including gates and failures nested in
`implementation_progress`. Validate task, branch, stage and referenced files against Git and the
plan. Older state may omit `proposal_path`; resolve it from the entry point, proposal and history.
With no live file, inspect the current branch, active assignments, `000-prompt.md`, implementation
log, commits and any parked handoff before inferring a stage. Report unclear or inconsistent state
before reconstructing it. Treat a saved `paused` stage as a stop until the user resumes it, then
confirm the resume point and any still-pending approval. A parked handoff belongs only to its recorded branch and task. Look for
`workflow-handoffs/` in the per-worktree Git directory from `git rev-parse --absolute-git-dir`. After
checking out the matching branch, restore the verified handoff only if root state is absent. Keep
the saved copy until the live state has been restored and validated.
Never stage the live state file.

Minimum state shape (populate real values):

```json
{
  "version": 1,
  "stage": "plan",
  "stage_detail": "",
  "task_id": null,
  "work_number": null,
  "branch": null,
  "plan_directory": null,
  "proposal_path": null,
  "complexity": null,
  "skipped_stages": [],
  "auto_advance": false,
  "refinement_history": [],
  "sequenced_stages": [],
  "implementation_progress": {"current_stage": null, "completed_stages": [], "failed_stages": []},
  "quality_pass": {},
  "docs_check": {},
  "pr_url": null,
  "last_updated": null
}
```

Stages in order are `plan`, `refine`, `fracture`, `singletonize`, `loss_check`, `sequencing`,
`implementing`, `testing`, `staging`, `review_gate`, `finalization` and `complete`. Update state
after each completed action using an atomic file replacement and a real ISO-8601 timestamp. Reconcile
recorded progress with commits, PR state and keitora before retrying an interrupted mutation. Record
failures without advancing the stage.

On invocation, summarize task/branch/stage and the concrete next action. With `auto_advance: false`,
obtain stage approval unless the user's current or earlier instructions already authorize it. Once a
stage is authorized, complete its work without asking again for its routine substeps. Explain a pending
stage gate as a requirement of this skill and cite this file. Offer skipping or auto-advance when useful.
Auto-advance permits ordinary stage transitions. Stop on entry to `auto_advance_until` if recorded,
and preserve plan-specific gates, fresh Refine input, human review approval and authorization for
merge/history rewrite. Record requested skips and their validation consequences. Pause at human gates
and continue through routine substeps. On a plan change, return to Refine and mark affected work for
revalidation.

## 1. Plan

Clarify unresolved intent and integration decisions; research real code and contracts. Write a proposal
with Goal, Scope (in/out), Architecture, Technical Design, Dependencies, Risks and Open Questions.
Keep it at design level and cite actual paths/symbols. Recommend complexity: small (fewer than 5 files,
one subsystem) skips fracture/singletonize/loss check; medium (5–20 files) keeps singletonize with optional
fracture and light loss check; large/cross-cutting keeps all. Honor user overrides and record skipped
stages. Create the task branch when safe, record it in keitora and commit the proposal. Next: `refine`.

## 2A. Refine

Iterate using fresh user feedback. Offer code-reference verification, a diagram if useful, direct
feedback, or marking refinement complete. Do not manufacture feedback in an automated loop. Record
cycles in `refinement_history` and commit updates. Exit only when the user marks refinement complete.
Next: first unskipped stage among fracture, singletonize, loss_check, sequencing.

## 2B. Fracture (optional)

Split the proposal into self-contained concern/subsystem documents under the plan directory, keeping
the original proposal for comparison and adding cross-references. Commit; next unskipped stage.

## 3. Singletonize (optional)

Verify references, expose assumptions and fill implementation gaps in each document without silently
redesigning it. Make each document independently implementable. Commit; next unskipped stage.

## 4. Loss check (optional)

Compare proposal, refinement history and Git history with the final documents. Record preserved,
justifiably modified, intentionally dropped and unintentionally lost requirements. Restore accidental
losses or resolve scope changes with the user. Commit the report/restorations; next: `sequencing`.

## 5. Sequencing

Write `000-prompt.md` (proposal and contract references, branch, stage list, tests and user gates)
and ordered stage files with Goal, Prerequisites, Implementation, Validation and Commit message.
Identify dependencies, shared files, any disjoint parallel work and the evidence required at each
gate. Test each logic-bearing unit alongside implementation. Mark temporary execution files for
Finalization and name the design records to keep. Commit; next: `implementing`.

## 6. Implementing

Resume from `implementation_progress` and the plan's implementation log, if present. Read
`000-prompt.md` and the relevant stage and binding interfaces before dispatching. Extend existing
systems. Include meaningful happy-path, boundary and error-path coverage with new behavior. Validate
documentation and mechanical changes with checks appropriate to the change. Coordinate one Cargo
command at a time across all agents in the worktree. Log validation, deviations, evidence and plan
gate verdicts. Honor user gates before proceeding and record commits and completed or failed stages
as they happen. Retry failed agent work once with better context, then handle it directly or report
the blocker. Next: `testing` when all stages are done.

## 7. Testing

Fetch and rebase on latest `origin/main` with a clean task-owned tree. If no remote main exists,
validate against local `main` and leave publication pending. Investigate unexpected commits or
conflicts and preserve user work. Run the relevant canonical build, test, typecheck and lint commands
from `AGENTS.md`. For frontend/runtime changes these include `bun run build`, `bun test`,
`bun run check` and `bun run lint`. For Rust changes these include `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all --check`. Use the plan
and affected surfaces to select applicable slow harnesses, parity, performance and reference checks.
For docs or skills alone, validate syntax, links and affected behavior. Record actual results and blockers.

Use focused checks while editing. Run the full applicable gate once when each testing or staging pass
is ready, after collecting its fixes. If it fails, fix the failure and rerun the gate. Pass the same
cadence to agents. Run a relevant demo or smoke test for runtime changes using the repository's app
or frontend launch commands. For visual changes capture and inspect the result, and check logs for
introduced errors. Record any required user acceptance evidence. Commit fixes; advance to `staging`
after required checks pass.

## 8. Staging

Run both local quality skills before pushing:

1. Read [simplify](../simplify/SKILL.md); apply it to the entire task diff.
2. Read [code-review](../code-review/SKILL.md); use its `high` thoroughness mode on the updated task diff.

With manual advancement, the approval to stage should cover the quality pass and any requested
publication. Honor explicit skips and record which passes were skipped and why.
The target is the merge-base diff against `origin/main` (or the local base used in Testing) plus
task-owned staged, unstaged and untracked
changes. Record the base and reviewed HEAD. Independently verify review findings, fix real defects,
justify dismissals and file deferred work. Commit focused refactor/fix changes and rerun affected
validation; repeat required pre-push checks if their prior result no longer covers the final tree.
Re-review affected paths after fixes. Record scope, outcomes, fixes/skips, validation and any lack of
reviewer independence under `quality_pass`. Never mark an unavailable or unperformed pass successful.

Then run the docs gate before pushing:

1. List touched areas from `git diff --name-only <base>...HEAD` using the review base ref and update the relevant
   `README.md`, `AGENTS.md`/`CLAUDE.md`, plan docs and any applicable `docs/` content, including
   `docs/build-plan.md` if present. Follow documented checks and paired changes from `AGENTS.md`,
   such as updating the embedded CLI skill when commands, flags, output or exit codes change.
2. File or update deferred keitora tasks. Commit docs changes as `docs(scope): ...`.
3. Record docs touched or the reason no updates were needed, task IDs and check results in
   `docs_check` and in the PR body's Docs section.

Respect existing authorization to push/open a PR; if missing, finish the local reviewable result first
and ask once for the pending remote action. Verify `origin` and GitHub CLI availability/authentication.
Look up an existing PR for the exact task branch before creating one, including after an interrupted
push. Push the unsquashed task branch and create or update the PR with a Conventional Commit title.
Cover the problem, result, plan references and actual validation in the body. Use a body file for
multiline `gh` text.
Do not add Claude attribution or invented author trailers. Attach the PR using the Codex app's
`attach_artifact` tool when available. Record `pr_url` in state and set it in keitora with
`keitora task update <id> --branch <branch> --pr <url>`. Next: `review_gate`.

## 9. Review gate

Report the PR and outstanding risks. Inspect `gh pr view` state, head, reviews and checks, and record
the approved head and any plan-specific acceptance evidence. Human approval of the current result
is required. Check whether approval authorizes finalization and merge. Next: `finalization` after approval.

## 10. Finalization

First check whether the PR already merged. Verify the merged tree contains the final task snapshot,
retained design records and planned stage-file removals before resuming bookkeeping alone. If an
external merge omitted finalization changes, report the gaps and finish them through the normal PR
path with the required authorization before closing the task.

Retain proposal, design/interface docs, loss report and `000-prompt.md`; remove only temporary execution
stage files identified during sequencing. Do not delete a directory containing retained design docs.
Update design/build-plan status before the final commit. Do not invent coauthor identities; follow
repo/user attribution conventions.

Push finalization commits normally, verify the PR contains them, and check review status and required
checks on the new head. Seek fresh human approval for material changes or if branch protection has
invalidated the prior approval. Merge via
`gh pr merge --squash --match-head-commit <verified-head>` with a Conventional Commit subject and
body. The server performs the squash; keep the review commits intact until merge. If a rebase is
required by the merge policy, confirm authorization for any remote history rewrite and revalidate
the resulting head before merging.

Merge through the PR, never push directly to main. Verify the merge before completing the task.
Update local main only if it is this checkout's branch and can fast-forward safely; otherwise leave the
checkout intact and report the merged PR. Never use `reset --hard` to synchronize a user's checkout.
Verify branch and PR metadata in keitora and run `keitora task done <id>` if the task is still active.
Report newly unblocked tasks. If tracking fails after merge, record pending tracking work and retry
only that work on resume.
Mark complete and delete local state only after finalization and task bookkeeping succeed.
