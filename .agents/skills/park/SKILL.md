---
name: park
description: Park the current Strudeland checkout on a stable per-worktree branch at the latest main commit while preserving unfinished work. Use when explicitly asked to park or return a worktree to main.
---

# Park this checkout

Invoke with `$park`. Park only the current user-created checkout. Preserve its ability to resume a
task in [workflow](../workflow/SKILL.md).

## Inspect before changing Git state

Read applicable `AGENTS.md`, then inspect:

- Repository root, current branch (or detached HEAD), HEAD SHA and `git worktree list --porcelain`.
- `git status --porcelain=v1 --untracked-files=all`, including staged and untracked work.
- Any rebase, merge, cherry-pick or revert in progress. Stop if one exists; do not abort user operations.
- Local `WORKFLOW_STATE.json`, if present, and the task it identifies. This ignored file can belong to
  unfinished work even when Git reports a clean tree. Keep the task claim and preserve its state.
- `origin` and its fetch URL. Fetch `origin` successfully, verify `origin/main` resolves and record its
  SHA. When there is no origin, or a successful remote lookup confirms it has no main branch, use
  local `main` and report which base was used. If the fetch or lookup fails, stop and report the
  failure. If no base has a commit yet, report that the checkout cannot be parked.

Use the same stable branch as Claude, `llm/{username}/{worktree-name}-parking`. Take the first word
of `git config user.name` and the worktree directory's basename and lowercase both. Validate with
`git check-ref-format --branch`. If the shared branch is absent and this checkout has an existing
`codex/{worktree-name}-parking` branch, reuse it. Keep the name stable and respect Git's worktree
ownership checks.

Compare both the current and target parking tips with the selected main SHA. Use ancestry first.
After a squash merge, verify the PR merged into main and that its merged head covers the local tip,
or verify equivalent changes from the local commit range. Inspect commits added after that head
separately. If landing is uncertain, treat the work as unfinished and keep its tip. Check without
rebasing the user's branch.

## Preserve unfinished work

If there are dirty files, unmerged commits, detached commits, unique commits on the parking branch,
or active workflow state, report the concrete paths/SHAs/task and resolve their disposition before
moving the checkout. Honor an already explicit preservation/discard instruction; otherwise ask once
whether to preserve and park, leave the checkout as-is, or discard the specifically listed work.
Explain that this pause comes from this skill's preservation requirement and link this `SKILL.md`.
Silence is not permission to discard.

For preserve-and-park:

- Retain committed work on its existing feature branch. Before replacing a parking tip or leaving a
  detached HEAD, create a named backup branch at its exact SHA and verify it. Use a user-specified name
  or a unique `codex/preserved-{worktree}-{timestamp}` name and report it.
- Save staged, unstaged and untracked files with `git stash push --include-untracked -m <unique-message>`.
  The stash stack is shared across worktrees. Find and verify the entry by its message and base,
  then report recovery by object ID with `git stash apply --index <oid>` on the original feature
  branch or the named backup of a displaced tip.
  If the user or plan requires WIP commits, preserve the index and final file contents separately
  so a file with both staged and unstaged edits can be recovered in full. Verify the checkout is
  clean before moving it.
- Preserve active `WORKFLOW_STATE.json` separately with its original branch and task association before
  moving. Save a uniquely named handoff under `workflow-handoffs/` in the per-worktree Git
  directory (from `git rev-parse --absolute-git-dir`). Verify the
  saved bytes. Clear the root state only after the switch succeeds and the saved and live bytes
  still match. Report the path and how to restore it to the root after checking out the saved
  branch. If a recovered tip needs a new branch name, reconcile the state branch field on resume.
  Keep other ignored files outside the backup.

Discard only the exact work the user explicitly authorized discarding; do not use a broad `git clean`
or erase unrelated/ignored files. If preservation fails, stop
before switching and report any backup or stash already created.

## Move to latest main

Once unfinished work has been resolved and the checkout is clean, recheck HEAD, the parking ref and
branch ownership. Use `git switch -c <parking> <main-sha>` for a new branch. For a verified existing
branch, `git switch -C <parking> <main-sha>` updates its tip only if the switch succeeds. Pass
`--no-overwrite-ignore` to either switch to protect ignored files.
Leave feature branches, local `main`, the remote and task status alone.

Verify the current branch is the intended parking branch, HEAD equals the selected main SHA,
and tracked/untracked status is clean. If concurrent Git changes invalidate the checks, inspect again
before proceeding. Report the branch, main SHA and preservation locations/recovery instructions, plus
any ignored files intentionally left in place. A blocked or partial operation is not a successful park.
