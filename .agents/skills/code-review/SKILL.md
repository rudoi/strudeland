---
name: code-review
description: Review a branch, working tree, commit or PR for actionable correctness regressions. Supports a high thoroughness mode for Strudeland's pre-push workflow; reports findings without editing by default.
---

# Code review

Invoke with `$code-review` and an optional target (for example, `$code-review high against origin/main`).
`high` selects the deeper procedure below and routes the independent reviewer to a stronger model
where the runtime supports it. The parent uses its configured settings.

## Scope

Honor explicit scope. Otherwise inspect local staged/unstaged/untracked changes, or the merge-base
branch diff against `origin/main` if clean. In the delivery workflow cover the entire task branch plus
its task-owned local changes. State base, HEAD and coverage. Inspect applicable `AGENTS.md`, binding
interfaces, full changed files, callers and tests. Verify the base exists; never silently substitute a
last-commit review. For a supplied PR, use available PR tools or `gh` and inspect the actual diff and
head; attach it in Codex when the app tool is available. Do not publish review comments without explicit
user authorization.

## Review

Prioritize introduced correctness bugs: broken contracts, edge/error paths, state transitions, resource
lifetime, unsafe/FFI invariants, concurrency, determinism, regressions and missing behavior coverage.
Check relevant Strudeland contracts, including Tauri/CLI protocol parity, legacy project data,
real-time audio constraints, deterministic render parity and generated host inputs.
Do not report style preferences, speculative problems without a reachable trigger, or pre-existing bugs
unless the change newly exposes them.

In `high` mode account for every changed area and trace affected behavior end-to-end, including
unchanged consumers. Check boundary values, cleanup and failure behavior, and configuration or
feature combinations relevant to the patch. Compare tests
with the actual contracts and run focused repros/checks where useful. For skills/workflow changes, trace
realistic invocation, resume and failure scenarios, including dependency paths and authorization gates.

When allowed, the parent delegates a substantive high-mode review to an independent reviewer using
`gpt-6.2-astra` at `high` effort when supported. Assigned reviewers inspect their scope directly and
return findings to the parent. Follow the [routing guide](../workflow/references/agents.md) for
assignment context and fallback. Give the reviewer the raw diff, contracts and scope without
seeding hypotheses. Add disjoint reviewers only when coverage warrants it. Verify findings against
the call path, tests or repro and check the union of reviewed areas yourself. If delegation is
unavailable, complete the high-mode procedure directly and disclose that the review was in-session.

## Output and fix boundary

Report only findings the author would act on, ordered by severity: P0 critical/unconditional, P1 urgent,
P2 normal defect, P3 minor actionable defect. Give each a concise title, exact file and tight changed-line
location, triggering conditions, concrete impact and supporting evidence. Keep uncertainty explicit;
ask an unresolved design question rather than inventing a defect. Emit Codex inline code comments where
supported. If no findings, say so and state material coverage/test limits. Do not invent findings to
fill a quota or call unexecuted tests passing.

Review is read-only by default. If the user or enclosing workflow authorizes fixes, first establish
findings, then fix verified defects with appropriate regression coverage, rerun affected checks and
re-review changed paths. Clearly separate fixed, dismissed and deferred findings. Do not commit, push,
merge or post external comments without authorization from the user or enclosing workflow.
