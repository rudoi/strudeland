# 0134 implementation entry point

- **Branch:** `llm/andrew/strudeland/0134-engine-interface`.
- **Base:** `7c6e632`. Diff against it, never against a local `main`, which may be stale. The
  branch's commits before stage 001 are plan documents only
  (`git diff --name-only 7c6e632 HEAD` lists only `plans/`).
- **Design:** [proposal.md](proposal.md) (the `Rendered` shape, where each text is built, the
  dead-code finding).
- **Evidence:** [reverify.md](reverify.md), with file and line at `7c6e632`. Its **Decisions**
  section is binding. Read §2 (the unsupported shapes) and §5 (the tests that break) before you
  edit anything.
- **Background reading:**
  - `AGENTS.md`: the engine bullet (`:231-254`) and the Rules on the Rust engine (`:423-442`);
  - `crates/strudeland-engine/src/lib.rs` (whole file, 96 lines);
  - `crates/strudeland-engine/src/capability.rs:562-722` (signatures and detection);
  - `crates/strudeland-engine/src/render.rs:86-125` (`run`, `run_observing_overflow`) and its
    tests `:376-396`, `:1639-1654`;
  - `crates/strudeland-engine/src/bin/strudeland-engine.rs:100-156` (`run_render`);
  - `crates/strudeland-engine/tests/orbit_pairs.rs` (62 lines) and `src/orbit.rs:815-830`
    (the unit test module's head);
  - `crates/strudeland-render/src/render.rs:200-247` (`render_one`) and `:328-420` (the fake);
  - `crates/strudeland-render/src/pipeline_audio.rs:100-215`, `:318-345`.

## Rules

- **No visible change, no sound change.** All of these stay byte-identical: the engine binary's
  `signatures` JSON, its `render` summary (key order, `ok`, `errors` entries and their order,
  messages including the trailing `keys=` and the 300-character cap), stderr, exit codes and
  WAV bytes, `controls --json`, the render crate's refusal and failure texts, the CLI, the
  export dialog and the live `Event::Warning` payload. No DSP code changes. The A/B capture, the
  literal script and the existing pinned tests prove it.
- **Only these files change** (paths under `crates/`):
  - `strudeland-engine/`: `README.md`; `src/` `lib.rs`, `capability.rs`, `delay.rs`, `djf.rs`,
    `live.rs` (its line-2 module doc only), `orbit.rs`, `param.rs`, `reverb.rs`, `render.rs`,
    `voice.rs`, `bin/strudeland-engine.rs`; `tests/` `bin.rs`, `fixtures.rs`, `micro.rs`,
    `never_panics.rs`, `stems.rs`, and `orbit_pairs.rs` (deleted);
  - `strudeland-render/`: `src/` `pipeline_audio.rs`, `plan.rs`, `render.rs`; `tests/`
    `decay_bound.rs`, `joiner_identity.rs`.

  Everything else is frozen. That includes `job.rs` (`ErrorKind`, `RenderError`, `MAX_MESSAGE`),
  every line of `live.rs` but its doc line 2, `stems.rs` (engine `src/`), the render crate's `error.rs` and `export/`, every
  other crate, every TS file, `AGENTS.md` and `SKILL.md`.
- **Edits outside the plan's list are forbidden.** In the `#[cfg(test)]` files
  (`delay.rs`, `djf.rs`, `param.rs`, `reverb.rs`, `voice.rs`, and the accessor part of
  `orbit.rs`), the only edits are the 32 `#[cfg(test)]` lines in 001's table and the three
  deletions it names. The two doc-only edits (`live.rs:2`, `capability.rs:452`) keep rustdoc
  from linking to now-private items and change no code.
- **Existing assertions keep their strength.** A test that used `errors` to detect a refused or
  unclean render also checks `unsupported` (001 step 6). No expected text in a test changes,
  except the `render_one` fake's message, which now comes from a signature (001 step 7).
- **New logic ships with tests** in the same commit: the engine unit tests assert the new field,
  and the `render_one` cap gets one test.
- **Run `cargo fmt --all`** before the commit. There are no frontend changes, so no
  `bun run format`.
- **Test cadence.** While working, run only `cargo test -p strudeland-engine --lib`, a single
  integration test (`--test fixtures`), or `cargo test -p strudeland-render --lib`. When the
  stage is done, run the full gate once, in order. Log each command and read the log instead of
  re-running it.
- **Never start the app** and never start playback.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan. A later session sets `S` to its own
scratchpad and recreates the scripts from 001's appendix. `L` logs a command with its start
time and exit code:

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
L() { log="$S/0134-001-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
```

| Step | Command | Pass |
|---|---|---|
| 1 | `L fmt cargo fmt --all --check` | exit 0 |
| 2 | `L clippy cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| 3 | `L clippy-win cargo clippy -p strudeland-engine --all-targets --target x86_64-pc-windows-msvc -- -D warnings` | exit 0. If it fails before compiling the engine for a reason unrelated to the code (a missing target component), record the error and go on. Any engine warning or error is a failure |
| 4 | `L test cargo test --workspace` | exit 0. The log shows `orbit::tests::the_table_covers_wrapping_and_non_positive_orbits`, `orbit::tests::orbit_pair_matches_the_shared_table` and `orbit::tests::stems_channels_route_to_the_pair` as `ok`, and no `Running tests/orbit_pairs.rs` |
| 5 | `L bun-test bun test` | exit 0. No TS changes. This shows that the TS fakes of the engine summary (`tests/render-rust-engine.test.ts`) and the staleness tests (`host.inputs.json`, refs provenance) still pass |
| 6 | `L ab-after "$S/0134-ab.sh" after`, then `L ab-diff diff -r "$S/0134-ab/base" "$S/0134-ab/after"` | both exit 0. `ab-after`'s summary line has the same counts as `ab-base` (001 step 1). `ab-diff` prints nothing |
| 7 | `L surface-after "$S/0134-surface.sh" after`, then `L surface-check "$S/0134-surface-check.sh"` | both exit 0. `surface-check` prints `removed 121, added 13` and `after 256` |
| 8 | `L literals "$S/0134-literals.sh"` | exit 0: no `FAIL` row |
| 9 | `L touched git diff --name-only 7c6e632 HEAD -- . ":(exclude)plans"` | exactly the 23 paths in Rules, sorted by git |

Notes:

- No `PLANNER_FILES`, no file in `host.inputs.json` and no IPC command changes. So no
  `engine:refs`, `engine:vectors`, `host:build`, `ipc:build` or `render:check` is needed.
  Step 5's staleness tests would fail otherwise.
- `cargo +nightly` is used only by the surface listing, with its own `CARGO_TARGET_DIR` under
  `$S`. The workspace still builds on stable.
- **The stage report lists:** the exit line of every log; the `ab-base` and `ab-after` summary
  lines; the `surface-check` output; the literal table from step 8; the names of the changed
  and new tests; and the clippy item list from 001 step 3 if it differed from the table (it
  must not).

## STOP, and report to the user, on any of these

Do not work around them:

- any A/B difference (`ab-diff` non-empty), or `ab-after` counts different from `ab-base`;
- any change to a text, JSON, exit code or WAV byte, or a DSP edit;
- a removal from the public surface outside `capability`, `orbit` and the 5 `Engine` or
  `quantum_end_time` lines, or an addition other than the 13 planned lines;
- clippy dead-code findings other than the 35 items of 001 step 3 (the 32 table rows and the 3
  deletions), after the two visibility edits (fewer, more or different);
- a literal count different from its column;
- an existing test whose expected value must change, except the planned `render_one` fake
  message;
- a needed edit to a frozen file;
- `include_str!` of the shared orbit table failing to resolve from `src/orbit.rs`.

Never loosen a check, and never re-record an expectation, to absorb a difference.

## Commits

- One commit, with the message the stage file gives. Use plain Conventional Commits with no
  task or plan identifiers. End it with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`) and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed.
