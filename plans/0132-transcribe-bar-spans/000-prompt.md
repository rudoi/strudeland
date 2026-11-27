# 0132 implementation entry point

- **Branch:** `llm/andrew/strudeland/0132-transcribe-bar-spans`.
- **Base:** `4af3182`. Diff against it, never against a local `main`, which may be stale. The
  branch commits on top of it before stage 001 are plan documents only
  (`git diff --name-only 4af3182 HEAD` lists only `plans/`).
- **Design:** [proposal.md](proposal.md). The `Options.bars` field, the window rule, the "why
  `bars` output matches today" argument and the stage rationale are there. Only `last_bar`
  becomes `bars`; there is no `SpanRequest` and no `Placement` ([reverify.md](reverify.md),
  Decisions).
- **Evidence:** [reverify.md](reverify.md), with file and line at `4af3182`. Its **Decisions**
  section is binding. Read §1 (the round trip), §2 (callers), §3 (outputs that must stay
  identical) and §4 (tests) before you edit anything.
- **Coverage:** [loss-check.md](loss-check.md) maps every proposal item to the stage that does
  it.
- **Background reading:**
  - `AGENTS.md`: the transcription layout bullet (`:332-339`) and the Transcription rule
    (`:492-500`: the eval pins are never loosened);
  - `crates/strudeland-transcribe/src/lib.rs:49-305` (`Options`, `transcribe`, `span_bar`) and
    its tests at `:446-576`, `:598-694`, `:840-905`, `:964-1054`;
  - `crates/strudeland-transcribe/src/notation.rs:234-278` (`render_notation`),
    `suggest.rs:1-171`, `load.rs:19-147`, `grid.rs:101-118` (`bar_span`) and `:197-300`
    (`resolve_grid`);
  - `crates/strudeland-cli/src/run.rs:549-680` (`transcribe_failure`, `build_hint`,
    `run_transcribe`, `resolve_span`) and the transcribe tests (`:2338-2900`);
  - `crates/strudeland-cli/src/cli.rs:394-470` (`TranscribeArgs`, `parse_memo_bars`).

## Rules for every stage

- **The one visible change is the fix.** For `--bars A-B` the notation starts at A, where
  today it sometimes starts at A-1. Everything else stays byte-identical for every input:
  - human and `--json` stdout, and `--debug` frames and onsets;
  - stderr warnings and their order;
  - exit codes, and the exit 1 and exit 4 texts (`BadSpan` stays worded in seconds);
  - the `MAX_NOTATION_BARS` warning text;
  - the JSON field set. No field is added or renamed.
- **Eval is frozen.** `tests/eval.rs`, `src/eval/*`, the README eval tables (`README.md:35`
  onward) and `consts.rs` are never edited. The eval table and the per-case eval dump must be
  byte-identical to the base.
- **Frozen files.** Neither stage edits these:
  - everything under `crates/strudeland-transcribe/` except `src/lib.rs`, `src/notation.rs`
    (001), `src/suggest.rs` (002) and `README.md` lines 1-34;
  - everything under `crates/strudeland-cli/` except `src/run.rs` (001). `skill/SKILL.md`,
    `cli.rs`, `output.rs` and `tests/` are frozen;
  - `Cargo.lock`, every `Cargo.toml`, and everything outside these two crates except
    `AGENTS.md` (001: the transcription bullet; 002: the one word on `:333`) and
    `plans/0132-transcribe-bar-spans/`.
- **Tests keep their assertions.** `Options` keeps `span_start_seconds` and `memo_seconds`, so
  only one existing test changes: the three `last_bar` settings of
  `a_requested_last_bar_keeps_trailing_rest_bars` (`lib.rs:641`, `:652`, `:659`) become
  `bars` settings. No other field, expected value, literal or assertion changes, in that test
  or any other. The new regression tests are frozen once their red run is logged.
- **New logic ships with tests** in the same commit.
- **Run `cargo fmt --all`** before each commit. No frontend file changes. If a stage needs
  one, STOP.
- **Never read the user's real projects or memos.** Every memo is synthetic, written by the
  scratch generator under `$S`.
- **Test cadence.** While working, run only the targeted tests the stage file names. When the
  stage is done, run the full gate once, in order. Log each command and read the log rather
  than re-running it.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan. A later session sets `S` to its own
scratchpad and regenerates the baseline there (001 steps 1-6) before any code edit. `N` is
the stage number (`001` or `002`). `L` logs a command with its start time and exit code:

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
L() { log="$S/0132-$N-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

| Step | Command | Pass |
|---|---|---|
| 1 | `L fmt cargo fmt --all --check` | exit 0 |
| 2 | `L clippy cargo clippy --locked --workspace --all-targets -- -D warnings` | exit 0 |
| 3 | `L transcribe-test cargo test --locked -p strudeland-transcribe --features eval` | exit 0. Lib tests 208 at base, plus the stage's new ones. The 6 eval tiers pass |
| 4 | `L cli-test cargo test --locked -p strudeland-cli transcribe` | exit 0 |
| 5 | `L cargo-test cargo test --locked --workspace -- --skip dropout_short` (CI's form) | exit 0 |
| 6 | `L perf cargo test --locked --release -p strudeland-transcribe --features eval --test perf_budget -- --ignored --nocapture` | exit 0 (median under 1 s) |
| 7 | `L eval sh -c 'cargo run --locked --release -q -p strudeland-transcribe --features eval --example eval > "$S/0132-$N-eval.txt" && cmp "$S/0132-base-eval.txt" "$S/0132-$N-eval.txt"'` | exit 0 |
| 8 | `L evaldump sh -c '"$S/0132-gen/run-evaldump.sh" "$S/0132-$N-evaldump.jsonl" && cmp "$S/0132-base-evaldump.jsonl" "$S/0132-$N-evaldump.jsonl"'` | exit 0 |
| 9 | `L grid "$S/0132-gen/grid.sh" $N` (001 step 2) | exit 0. 001: the classifier passes. 002: zero differences from 001 |
| 10 | `L frozen "$S/0132-frozen.sh" $N` (001 step 2) | exit 0 |
| 11 | `L literals "$S/0132-literals.sh" $N` (001 step 2) | exit 0 |

Notes:

- `cargo test --workspace` needs cmake, a C compiler and the V8 download. The first build takes
  minutes.
- Step 8 rebuilds the scratch generator against the working tree. Its sources use only API
  that neither stage changes (`eval::cases`, `serde_json`).
- **Each stage report lists:**
  - the exit line of every log;
  - the new test names;
  - the classifier's table (001) or the zero-diff count (002);
  - the literal-count table.

## STOP, and report to the user, on any of these

Do not work around them:

- any difference in the eval table or the eval dump, or a needed edit to `tests/eval.rs`, the
  README eval tables or `consts.rs`;
- a grid difference the classifier does not accept. In 002, any grid difference at all;
- a predicted hit that the base output contradicts (001 step 5). The plan's arithmetic would
  then be wrong;
- a test that needs a changed expected value, literal or assertion to pass, or a regression
  test that passes on the base code (its red run must fail);
- a literal count that differs from the table;
- a needed edit to a frozen file, `SKILL.md` included;
- a perf budget failure.

Never loosen a check, and never re-record an expectation, to absorb a difference.

## Commits

- Use plain Conventional Commits, with the message each stage file gives and no task or plan
  identifiers. End each one with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed.
