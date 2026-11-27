# 0144 implementation entry point

- **Branch:** `llm/andrew/strudeland/0144-length-errors`.
- **Base:** `0113154`. Diff against it, never against a local `main`, which may be stale. Before
  stage 001, the branch's commits on top of the base touch only `plans/`
  (`git diff --name-only 0113154 HEAD`).
- **Design:** [proposal.md](proposal.md) (the codes, the wire form, the dialog wordings and the
  gate). **Evidence:** [reverify.md](reverify.md). Its §1 lists every site and every test that
  pins a text. Its Decisions are binding.
- **Background reading:**
  - `AGENTS.md`: the headless host bullet, the render pipeline bullet (`:279-312`), the desktop
    export bullet (`:160-165`), and the Rules on the Rust engine refs, the headless host, the
    render pipeline, IPC and styling;
  - `src/lib/render/job-plan.ts:52-69` and `:310-520`;
  - `src/host/main.ts:37-41` and `:72-85`;
  - `crates/strudeland-host/src/error.rs` (all of it) and `host.rs:207-246`;
  - `crates/strudeland-render/src/error.rs:1-80`, `export/mod.rs:1-30` and
    `src/lib.rs:57-59`;
  - `src-tauri/src/export_audio.rs:143-173` and `:785-830`;
  - `src/lib/export/controller.ts`, `src/components/ExportAudio.svelte:28-80` and `:260-275`;
  - `docs/writing-contextual-help.md`.

## The contract

- **CLI unchanged.** The stdout, stderr, exit code and files written by `strudeland render` and
  `strudeland-render render` stay byte-identical for every input. The same holds for the stderr
  and exit code of `strudeland-host plan-render`, and its stdout plans. `SKILL.md` is unchanged.
- **No engine change.** Engine job texts, goldens, engine and render vectors, and ref audio stay
  as they are. The refs record only the new planner hash, plus the two floor cases'
  re-render (gate step 3).
- **No IPC change.** `bun run ipc:build` writes no diff. `ExportError` keeps its shape.
- **Visible changes:** only the dialog's wording of a length error (proposal, Dialog wordings),
  and the disabled export button for a loop preset without a marked loop, with its help.

## Stages

There are two stages. They run in order, and each one is one commit that is green on every gate
below.

1. [001-typed-length-codes.md](001-typed-length-codes.md): the typed codes from the TS planner
   to the dialog wording. This stage records the CLI and plan baselines **before any edit**,
   then carries every planner obligation.
2. [002-loop-preset-gate.md](002-loop-preset-gate.md): `loopUnavailable` and the dialog's
   disabled export. Frontend only.

They are separate stages for two reasons:

- 001 carries every generated-file obligation, and its proof is that nothing the CLI prints
  moved.
- 002 is a UI behaviour change with its own small diff. It touches no planner file or Rust
  file, so the baselines re-run unchanged as its control.

002 does not read anything 001 adds. It stays second so that the first commit holds the whole
typed-code change, as the Decisions' PR shape says.

## Rules for every stage

- **Baselines first.** Stage 001 step 1 runs on the untouched tree. Never re-record a
  baseline after the first code edit, and never edit an A/B script after its determinism pass.
- **Text-pinning tests keep every expected literal byte for byte.** Where a test builds or
  matches `HostError::MetaFlag` or reads `PlanJobError.flag`, only that construction or match
  changes (stage 001 lists each one). Its message and stderr literals stay the same. These pass
  unedited:
  - `crates/strudeland-host/tests/bin.rs`;
  - `crates/strudeland-render/tests/bin.rs`;
  - `crates/strudeland-cli/tests/e2e.rs`;
  - `crates/strudeland-cli/src/skill.rs`;
  - `crates/strudeland-host/tests/never_panics.rs`. Its `MetaFlag { .. }` pattern still
    compiles.
- **Generated files are written only by their commands:**
  - `crates/strudeland-host/js/host.js` and `host.inputs.json`: `bun run host:build`;
  - `crates/strudeland-engine/tests/refs/`: `bun run engine:refs --write`;
  - `src/lib/ipc/bindings.ts`: `bun run ipc:build`, which must write nothing.
- **New logic ships with tests** in the same commit.
- **Run `bun run format` and `cargo fmt --all`** before each commit. `bun run format` may
  rewrite files: stage them, then run `bun run lint`.
- **Test cadence.** While working, run only the targeted tests each stage file names. When the
  stage is done, run the full gate once, in order. Log each command once and read the log rather
  than re-running it.
- **No scratch worktrees** (`git worktree add`), no bare `git stash`, and never `git switch` away
  with edits in the tree. To set work aside, make a WIP commit. Never start the app.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
A later session sets `S` to its own scratchpad. If `$B` is then missing, rebuild the baseline
binaries from the base without touching the working tree:

```sh
mkdir -p "$S/base" && git archive 0113154 | tar -x -C "$S/base"
(cd "$S/base" && CARGO_TARGET_DIR="$S/base-target" cargo build --locked \
  -p strudeland-cli -p strudeland-render -p strudeland-host --bins)
```

Then copy the three binaries to `$B/bin/` and record as 001 step 1 says. The songs, case lists
and scripts are re-created from 001 step 1's text. `N` is the stage number (`001` or `002`).
`L` logs a command with its start time and exit code:

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
B="$S/0144-baseline"
L() { log="$S/0144-$N-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

| Step | Command | Stages | Pass |
|---|---|---|---|
| 1 | `L host-build bun run host:build` | 001, 002 | exit 0. In 002, `L host-unchanged git diff --exit-code HEAD -- crates/strudeland-host/js` exits 0 too |
| 2 | `L host-diff "$B/host-diff.sh"` | 001 | exit 0: every `host.js` hunk is inside the `job-plan.ts` region or a `main.ts` region, and `host.inputs.json` changes only those two hashes and `combined` |
| 3 | `L refs-check bun run engine:refs` | 001, 002 | 001: exit 1, and its summary names only provenance and the two floor cases (`mod-lfo-note`, `vowel-formants`). 002: exit 0 |
| 4 | `L refs-write bun run engine:refs --write` | 001 | exit 0 |
| 5 | `L refs-recheck bun run engine:refs`, then `L refs-ab "$B/refs-ab.sh"` | 001 | both exit 0 |
| 6 | `L engine-vectors bun run engine:vectors` and `L render-vectors bun run render:vectors` (check mode) | 001, 002 | exit 0 |
| 7 | `L frozen git diff --exit-code 0113154 -- <the stage's frozen list>` | 001, 002 | exit 0 (lists in each stage file) |
| 8 | `bun run format`, then `L bun-test bun test`, `L check bun run check`, `L lint bun run lint` | 001, 002 | exit 0 |
| 9 | `L ipc bun run ipc:build`, then `L ipc-diff git diff --exit-code -- src/lib/ipc` | 001, 002 | both exit 0 |
| 10 | `L build cargo build --locked -p strudeland-cli -p strudeland-render -p strudeland-host --bins` | 001, 002 | exit 0 (the A/B scripts use these debug binaries) |
| 11 | `L plan-ab "$B/plan-ab.sh" check "$S/0144-$N-plan"` | 001, 002 | exit 0, ending `equal: <n> cases` with the baseline's count |
| 12 | `L length-ab "$B/length-ab.sh" check "$S/0144-$N-length"` | 001, 002 | exit 0, ending `equal: 35 cases` |
| 13 | `L host-tests cargo test --locked -p strudeland-host` | 001, 002 | exit 0 |
| 14 | `L cargo-test cargo test --locked --workspace` | 001, 002 | exit 0 |
| 15 | `L clippy cargo clippy --locked --workspace --all-targets -- -D warnings` and `L fmt cargo fmt --all --check` | 001, 002 | exit 0 |
| 16 | `L render-build bun run render:build`, then `L parity bun run host:parity`, `L parity-batch bun run host:parity --batch`, `L parity-mm bun run host:parity --mixed-meter` | 001 | exit 0 each |

Notes:

- Steps 3-5 are the provenance rule. `job-plan.ts` is in `PLANNER_FILES`, and the floor cases'
  job files carry the provenance. `settleFloorCase` re-renders them, so their WAVs and
  `floorDb` may move. Nothing else may.
- There is no `perf_budget` step. Only throw sites change, and no path that plans a job does.
- No `render:check`. Job identity is proven by the plan A/B (host against base) and by
  `host:parity` (page against host).
- **Each stage report lists:**
  - the exit line of every log;
  - the A/B case counts;
  - the `refs-ab` counts (JSON files changed only in provenance, WAVs changed), in 001;
  - the `host-diff` hunk count per region, in 001;
  - in 001, the dialog text that `dialog_text` gives for D and K.

## STOP, and report to the user, on any of these

Do not work around them:

- any length A/B or plan A/B difference: a CLI text, exit code, written file, plan or host error
  text;
- a needed change to an expected literal in a text-pinning test;
- any `host:parity` difference, in any mode;
- a refs change beyond the provenance rule, or any golden or vector change;
- a `host.js` hunk outside the allowed regions, or any other `host.inputs.json` line;
- any diff from `ipc:build`, or a need to change `ExportError`, `CommandErrorKind` or a command
  signature;
- a need to touch `SKILL.md`, a CLI flag, `src/lib/strudel/structure.ts`, a threshold, or an A/B
  script after its determinism pass;
- a baseline case that does not give its expected code at baseline (001 step 1.8) after one
  song fix.

## Commits

- Use plain Conventional Commits, with the message each stage file gives and no task or plan
  identifiers. End each one with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- Nothing under `$S` is committed.
