---
disable-model-invocation: true
model: opus
---
strudeland creation workflow: take one task (or feature) from concept to a merge-ready commit through
staged, human-gated steps. Detects the current stage and executes the next, or starts fresh.

This is strudeland's full delivery pipeline. It is intentionally rigorous: the staged gates are the
guardrails that keep multi-session development coherent.

## Model routing for sub-agents
When dispatching sub-agents via the Agent tool, use the cheapest model that fits and pass `model`
explicitly on every call:

| Stage / task | Model | Why |
|---|---|---|
| Planning — codebase research, architecture | `opus` | deep reasoning |
| Refine — verification, doc editing | `sonnet` | structural, not creative |
| Fracture / Singletonize — doc splitting, gap-filling | `sonnet` | mechanical editing |
| Loss check — diff/checklist comparison | `haiku` | simple comparison |
| Sequencing — ordering, instruction writing | `sonnet` | moderate reasoning |
| Implementing (complex) — new systems, tricky logic | `opus` | full reasoning |
| Implementing (scaffold) — boilerplate, wiring | `sonnet` | routine code |
| Testing — run tests, straightforward debugging | `sonnet` | pattern matching |
| Staging / Finalization — git ops | `haiku` | mechanical |
| Explore sub-agents — search, grep | `haiku` | cheap |

When unsure whether a sub-agent task is "complex" or "scaffold," default to `sonnet`.

## Test cadence
Run the full test gate (the repo's canonical test, typecheck, lint and slow harness commands)
**once per stage**, when the stage's work is complete, not after every commit or fix within it. While
working, run only the targeted tests for the code being changed (one test file, one crate, one
fixture). A stage with several commits (an implementation stage, the /simplify fixes, the
/code-review fixes) makes all of them, then runs the full gate once; if it fails, fix and re-run.
Pass the same rule to sub-agents in their prompts.

## Source of truth
- `docs/build-plan.md` — the design/implementation plan, **if it exists**. When it does, read the
  relevant section before planning and update its status when finalizing. Code is the source of truth
  for implementation state; when the plan and code disagree, ask the user which should change.
- `keitora task` — the cross-worktree task CLI (installed at `~/.local/bin/keitora`).
  `keitora task list/show/new/claim/done/...`, `--json` everywhere.
- `AGENTS.md` / `CLAUDE.md` — repo conventions and the canonical build/test/lint commands, once they
  exist. If they don't yet, discover the commands from the project manifests and propose adding them.

## Work numbers, plans, branches
- **Work number = the task id**, 4-digit zero-padded (task `#1` → `0001`).
- Plan documents live in-repo at `plans/` (e.g. `plans/0001-first-feature/`).
- Branch: `llm/<user>/strudeland/{NNNN}-{kebab-desc}` off `main`.
- Derive once: `git config user.name` (first word, lowercased), worktree basename, the task id.

## Pre-workflow protocol
1. `keitora task list` — show claimable tasks. For a fresh feature not yet tracked,
   `keitora task new "title" -d "..."`.
2. `keitora task list --mine` — if this worktree already has an active task, ask whether to continue it.
3. `keitora task claim <id>` (fails if blocked by unfinished deps — respect it).
4. If `docs/build-plan.md` exists, read the matching section; flag any gaps or conflicts with the
   documented design.

## Follow-up tasks (every stage)
Whenever work is deferred — out-of-scope items in the proposal, review findings skipped for later,
TODOs discovered while implementing — file it immediately with
`keitora task new "title" -d "..." -t <tags>` and `keitora task block <new> <current>` when it depends
on the current task. Reference the task numbers in the plan docs (never in commits or code).

## Post-workflow protocol
After finalizing: `keitora task update <id> --branch <branch>` then `keitora task done <id>` (report
newly-unblocked tasks); update `docs/build-plan.md` status if present; update any design notes the
implementation changed.

---

# The pipeline

```
1.  Plan          — clarify intent, research, write a work proposal
2A. Refine        — iterate on the proposal with user feedback
2B. Fracture      — split the proposal into parallel-editable documents   (skippable: small)
3.  Singletonize  — harden each document to stand alone, unambiguous       (skippable: small)
4.  Loss check    — verify nothing was dropped between proposal and docs   (skippable: small)
5.  Sequencing    — turn the plan into ordered implementation stages
6.  Implementing  — execute the stages as commits (tests alongside code)
7.  Testing       — build + test + lint; run a demo; debug
8.  Staging       — /simplify + /code-review high, then push for review + open PR
9.  Review gate   — human reviews/approves
10. Finalization  — archive plan docs, squash, merge to main
```

## WORKFLOW_STATE.json (repo root, gitignored)
```json
{
  "version": 1,
  "stage": "plan|refine|fracture|singletonize|loss_check|sequencing|implementing|testing|staging|review_gate|finalization|complete",
  "stage_detail": "free-text: where within the stage we are",
  "task_id": 1,
  "work_number": "0001",
  "branch": "llm/<user>/strudeland/0001-first-feature",
  "plan_directory": "plans/0001-first-feature",
  "complexity": "small|medium|large",
  "skipped_stages": [],
  "auto_advance": false,
  "refinement_history": [],
  "sequenced_stages": [],
  "implementation_progress": { "current_stage": null, "completed_stages": [], "failed_stages": [] },
  "quality_pass": {},
  "docs_check": null,
  "pr_url": null,
  "last_updated": "ISO-8601"
}
```

## Invocation protocol
Every invocation: (1) detect the current stage from `WORKFLOW_STATE.json`; (2) state the next action;
(3) wait for approval unless `auto_advance` is true; (4) execute; (5) update the state file; (6) report
results + what's next.

## State detection
1. Read `WORKFLOW_STATE.json` (repo root). Trust `stage`; spot-check that `branch` matches the current
   branch and referenced files exist. If inconsistent, warn and offer to re-detect.
2. No state file → no workflow in progress. On a parking branch or `main`, check `keitora task list --mine`
   for an active task and offer to start; otherwise offer a fresh Stage 1.

## Complexity assessment (at the end of Stage 1)
- **Small** (<5 files, one subsystem): recommend skipping 2B/3/4; plan stays one document.
- **Medium** (5–20 files, some design decisions): keep Singletonize; Fracture optional; light Loss check.
- **Large** (cross-cutting, new systems): all stages.
Present the assessment; let the user override. Record `complexity` + `skipped_stages` in the state file.

---

## Stage 1 — Plan
Clarify intent (iterative Q&A — dig into edge cases and integration points, don't stop at the first
answer). Research the codebase for concrete references (use Explore sub-agents). Then write a work
proposal at `plans/{NNNN}-{desc}.md`: **Goal, Scope (in/out), Architecture, Technical Design** (referencing
real code), **Dependencies, Risks, Open Questions**. Keep it design-level (what/why), not line-by-line code.
Run the complexity assessment. Create the branch off `main`, commit the proposal
(`plan({scope}): {desc}`). **Exit:** proposal committed; state → `refine`.

> **Commit-message rule:** commit messages and source code MUST NOT carry plan/task identifiers
> (`[#NNNN]`, `#NNNN`, "Stage N" back-references, plan paths). Traceability lives in the branch name,
> the `plans/` docs, and the task CLI — never in the committed message or the code. Commits are plain
> Conventional Commits: `type(scope): desc`.

## Stage 2A — Refine
Iterate on the proposal **with fresh user input** (this is not an automated loop — that drifts toward
"code-in-the-plan"). Offer: run an automated review (Explore sub-agent verifies code references /
inconsistencies); generate an architecture sketch; take direct feedback; or mark refine complete. Record
each cycle in `refinement_history`, update + commit the proposal. **Exit:** user marks refine complete;
state → `fracture` (or `sequencing` if 2B skipped).

## Stage 2B — Fracture (skippable)
Split the proposal into self-contained documents along natural boundaries (subsystem/concern/phase) under
`plans/{NNNN}-{desc}/`, with cross-references. Commit. **Exit:** state → `singletonize`.

## Stage 3 — Singletonize (skippable)
Harden each document so a less-capable agent could implement it with zero ambiguity: verify every
referenced symbol/path exists, expose assumptions, fill gaps, replace vague language with decisions. This
is clarification, not redesign. Commit. **Exit:** state → `loss_check`.

## Stage 4 — Loss check (skippable)
Compare the original proposal + all refinement feedback against the final documents (use `git log`/`show`
and `refinement_history`). Produce a brief loss report: preserved / modified (justified) / intentionally
dropped / **unintentionally dropped** (restore these or mark out of scope with the user). Commit any
restorations. **Exit:** state → `sequencing`.

## Stage 5 — Sequencing
Produce ordered stage files in the plan directory: `000-prompt.md` (entry point: references the proposal,
lists stages, names the branch, states test-rigor expectations) plus `NNN-stage-name.md` files (Goal,
Prerequisites, Implementation, Validation, Commit message). Identify whether implementation is sequential
(test each stage before the next) or parallel (batch build checks, but never defer behavioral tests for
units with real logic). Commit. **Exit:** state → `implementing`.

## Stage 6 — Implementing
Execute the sequenced stages as focused commits, resuming from `implementation_progress`. Delegate stages
to sub-agents (with the stage doc + proposal as context) to preserve orchestrator context. **Write unit
tests alongside the code — core, not optional:** any new logic ships with test coverage in the same
change (happy path, edge cases, error paths). Untested new logic is not "done". Extend existing modules
rather than forking parallel implementations of the same logic. Verify each stage's validation before
committing (see **Test cadence**); update `implementation_progress`. Push through as many stages as possible per invocation.
**Exit:** all stages done; state → `testing`.

## Stage 7 — Testing
Rebase on latest `main`. Then run the repo's canonical commands (from `AGENTS.md`/`CLAUDE.md`, or the
project manifests): build clean; **tests pass and new logic added tests this task**; typecheck/lint
clean. **Run an example/demo that exercises the change** (use the `/run` skill if no project-specific
launch skill exists) and smoke-test it; for anything visual (UI, rendering, animation), capture a
screenshot and review it, and watch the logs for errors or warnings the change introduced. Debug
failures (sub-agents for hard ones);
commit fixes. **Exit:** green on rebased branch; state → `staging`.

## Stage 8 — Staging
**Pre-push quality pass.** Before pushing, run two review skills over the branch diff
(`origin/main...HEAD`) via the Skill tool:
1. `/simplify` — reuse/simplification/efficiency cleanups, applied to the working tree.
2. `/code-review high` — correctness review at high effort; then fix the verified findings that are
   real (skip or justify the rest).

If `auto_advance` is true, run both automatically. Otherwise ask the user first ("Run /simplify and
/code-review high before pushing?") and honour the answer. After each pass: re-run the test and
typecheck/lint commands if code changed, and commit the fixes as focused commits (`refactor(scope): …`
for simplify, `fix(scope): …` for review findings). Record what ran and what was fixed/skipped under
`quality_pass` in the state file.

**Docs gate.** After the quality pass and before the push, bring the repo's docs in line with the
change:
1. List the touched areas from `git diff --name-only origin/main...HEAD`.
2. Update whatever docs cover them: `README.md`, `AGENTS.md`/`CLAUDE.md` (new or changed
   build/test/lint commands, conventions), `docs/` if it exists, and `docs/build-plan.md` status.
   If the repo has a documented docs process or check command (see `AGENTS.md`/`CLAUDE.md`), follow
   it and make sure the check passes.
3. File or update follow-up tasks with `keitora task new`/`keitora task update` for anything deferred.
4. Commit as `docs(scope): ...`. If no docs change is needed, say why in the PR body.
5. Record `docs_check` in the state file: docs touched, tasks added or changed, and any check result.
   Put the same facts in the PR body under "Docs". The state file is deleted at Finalization, so the PR
   body is the lasting record.

Then push the **unsquashed** branch to `origin` (`git push -u origin {branch}`) — the commit history is the
review artifact — then open a PR via `gh pr create` (title `{type}({scope}): {desc}`, body with
summary + plan path + test plan + the 🤖 Generated-with-Claude-Code trailer). Record `pr_url` in the state
file. **Exit:** state → `review_gate`.

## Stage 9 — Review gate
Human gate. Report the branch/PR and what changed; wait for approval. If a PR exists, check
`gh pr view --json state,reviewDecision,statusCheckRollup`. **Exit:** approved → state → `finalization`.

## Stage 10 — Finalization
Keep the plan's design docs under `plans/{NNNN}-{desc}/` (proposal, fractured/singletonized docs,
loss report, `000-prompt.md`) — as-is, don't rewrite. Remove the working plan docs + stage files +
`WORKFLOW_STATE.json` from the branch in a commit. `git fetch origin main` and rebase + squash onto it;
write the final conventional commit (`{type}({scope}): {desc}`, ending with the current
`Co-Authored-By: Claude … <noreply@anthropic.com>` trailer). Then `git push --force-with-lease` and
`gh pr merge {pr} --squash --delete-branch --subject "{final subject}" --body "{final body}"`. Pull `main` forward locally
(`git checkout main && git fetch && git reset --hard origin/main`).
Then post-workflow protocol: `keitora task update <id> --branch <branch>`, `keitora task done <id>`, update
`docs/build-plan.md` status if present. **Exit:** merged; state → `complete` (delete the state file).

---

## User interaction protocol
Before each stage transition (unless `auto_advance`):
```
Current stage: {stage}
  {brief state}
Next action: {stage}
  {what will happen}
Proceed?
```
Options: proceed · skip to stage… (warn about skipped steps) · enable auto-advance · explain in detail.
On resume mid-workflow, summarize plan/branch/stage/task and offer: continue · review state · go back to a
stage · abandon (archive state, return to parking/main).

## Error handling
- Corrupt/inconsistent state → warn, offer heuristic re-detect or fresh start.
- Missing referenced files → check `git log`; ask before reconstructing.
- Branch diverged unexpectedly → `git log` to understand; ask about unexpected commits.
- Sub-agent fails in implementation → record, retry once with more context, then escalate.
- User changes the plan mid-implementation → valid; return to 2A Refine, note that completed stages may
  need revisiting; update the state file.

## Notes
- Remote: `origin` (check `git remote -v`; if none is configured yet, stop at Staging and ask the user to
  set one up). Staging/Review/Finalization go through real PRs. Never push straight to `main`; land via
  the PR in Finalization.

Now execute the workflow with the user's arguments: $ARGUMENTS
