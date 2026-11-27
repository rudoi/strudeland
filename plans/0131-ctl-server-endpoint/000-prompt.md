# 0131 implementation entry point

- **Branch:** `llm/andrew/strudeland/0131-ctl-server-endpoint`.
- **Base:** `9dbe6c5`. Diff against it, never against a local `main`, which may be stale. The
  branch commits on top of it before stage 001 are plan documents only
  (`git diff --name-only 9dbe6c5 HEAD` lists only `plans/`).
- **Design:** [proposal.md](proposal.md). The API, the text table and the stage rationale are
  there.
- **Evidence:** [reverify.md](reverify.md), with file and line at `9dbe6c5`. Its
  **Decisions** section is binding. Read §1 (what lives where), §3 (tests) and §4 (every
  visible text and behaviour) before you edit anything.
- **Background reading:**
  - `AGENTS.md`: the `strudeland-ctl` and `control.rs` layout bullets (`:179-184`,
    `:190-194`), and the Rules on the control socket (`:386-398`);
  - `src-tauri/src/control.rs:1-45` (imports), `:128-166` (`Bridge`), `:416-772` (the
    endpoint, `start`, `shutdown`), and the test module's socket and serve parts
    (`:1207-1237`, `:1670-1990`);
  - `crates/strudeland-ctl/src/windows_pipe.rs:120-153`, `:294-398`, `path.rs:88-139`,
    `client.rs:73-104`, `lib.rs`, `Cargo.toml`;
  - `src-tauri/src/lib.rs:24`, `:60`, `:123` (callers; unchanged).

## Rules for every stage

- **No visible change.** All of these stay byte-identical:
  - every detail text and stderr line (proposal, "Texts and where they are built");
  - the status strings `listening`/`in-use`/`disabled`;
  - the socket paths, the modes (socket `0600`, lock `0600`, new parent `0700`, an existing
    macOS parent untouched, the Linux parent required `0700`) and the lock file name
    `{path}.lock`, never removed;
  - the second-instance and stale-socket behaviour, and the silent Linux peer rejection;
  - the wire protocol: framing, `serve_connection`, `handle_line` and the wire examples.
- **Frozen files.** Neither stage edits these:
  - `crates/strudeland-ctl/src/{client,frame,methods,path,protocol,windows_pipe,wire_fixtures}.rs`
    and `crates/strudeland-ctl/tests/`;
  - `crates/strudeland-cli/` (`SKILL.md` and `e2e.rs` included);
  - `src/` (the frontend, `src/lib/ipc/bindings.ts` included);
  - `src-tauri/src/lib.rs`, `src-tauri/src/snapshot.rs`, `README.md`.
- **Text-pinning tests are frozen once written.** After 001 step 1 logs them green on the
  unchanged code, no expected literal, composition or assertion in a pin changes. That holds
  for the app copy and the ctl copy alike.
- **Move tests, don't rewrite them.** The bodies that move from `control.rs` into ctl stay byte
  for byte. That is why the private helpers keep their names.
- **New logic ships with tests** in the same commit.
- **Platform code belongs in ctl.** After 002, `control.rs` has no `cfg(unix)`, `cfg(windows)`
  or `target_os` outside test-only attributes. ctl's platform code is linted for macOS, the
  installed `x86_64-pc-windows-msvc` and the installed `x86_64-unknown-linux-gnu`, with
  `server` on and off.
- **Run `cargo fmt --all`** before each commit. No frontend file changes, so `bun run format`
  is not needed. Run it only if a stage unexpectedly touches one, and then STOP (frozen).
- **Test cadence.** While working, run only the targeted tests the stage file names. When the
  stage is done, run the full gate once, in order. Log each command and read the log rather
  than re-running it.

## The full gate

`S` is the scratchpad,
`/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad`.
That path belongs to the session that wrote this plan. A later session sets `S` to its own
scratchpad. `N` is the stage number (`001` or `002`). `L` logs a command with its start time
and exit code:

```sh
S=/private/tmp/claude-501/-Volumes-rudenet-code-strudeland-worktrees-poptart/2d85ebc3-84cd-4f6d-90b0-657c13b5df8a/scratchpad
L() { log="$S/0131-$N-$1.log"; shift; { date; "$@"; echo "exit $?"; date; } > "$log" 2>&1; tail -3 "$log"; }
# Cross-target clippy of ctl alone; its own target dir, so it never thrashes the main one.
X() { t=$1; shift; CARGO_TARGET_DIR="$S/xtarget" cargo clippy --locked -p strudeland-ctl --target "$t" "$@" -- -D warnings; }
# ctl's tests on Linux (aarch64), unprivileged, offline against the host registry.
DL() { docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e CARGO_TARGET_DIR=/tmp/t \
  -v "$PWD":/w:ro -v "$HOME/.cargo/registry":/usr/local/cargo/registry:ro -w /w \
  rust:1.100.0-bookworm cargo test --locked --offline -p strudeland-ctl "$@"; }
```

| Step | Command | Stages | Pass |
|---|---|---|---|
| 1 | `L fmt cargo fmt --all --check` | 001, 002 | exit 0 |
| 2 | `L clippy cargo clippy --locked --workspace --all-targets -- -D warnings` | 001, 002 | exit 0 |
| 3 | `L clippy-ctl-lib cargo clippy --locked -p strudeland-ctl --lib -- -D warnings` (`server` off: the CLI's build) | 001, 002 | exit 0 |
| 4 | `L x-win-lib X x86_64-pc-windows-msvc --lib`, `L x-win-all X x86_64-pc-windows-msvc --all-targets`, `L x-linux-lib X x86_64-unknown-linux-gnu --lib`, `L x-linux-all X x86_64-unknown-linux-gnu --all-targets` | 001, 002 | exit 0 each. `--all-targets` builds `server` (the self dev-dependency), `--lib` does not |
| 5 | `L ctl-test cargo test --locked -p strudeland-ctl` | 001, 002 | exit 0. The log lists the `server::` tests |
| 6 | `L ctl-test-linux DL` | 001, 002 | exit 0. The log lists the Linux-only `server::` tests |
| 7 | `L app-test cargo test --locked -p strudeland-app control::` | 001, 002 | exit 0 |
| 8 | `L cargo-test cargo test --locked --workspace -- --skip dropout_short` (CI's form) | 001, 002 | exit 0 |
| 9 | `L features sh -c 'cargo tree --locked -p strudeland-cli -e normal,features -i strudeland-ctl; echo ---; cargo tree --locked -p strudeland-app -e normal,features -i strudeland-ctl'` | 001, 002 | exit 0. Above `---`: no `feature "server"`. Below it: none in 001, present in 002 |
| 10 | `L frozen git diff --exit-code 9dbe6c5 -- crates/strudeland-ctl/src/client.rs crates/strudeland-ctl/src/frame.rs crates/strudeland-ctl/src/methods.rs crates/strudeland-ctl/src/path.rs crates/strudeland-ctl/src/protocol.rs crates/strudeland-ctl/src/windows_pipe.rs crates/strudeland-ctl/src/wire_fixtures.rs crates/strudeland-ctl/tests crates/strudeland-cli src src-tauri/src/lib.rs src-tauri/src/snapshot.rs README.md` | 001, 002 | exit 0 |
| 11 | `L literals "$S/0131-literals.sh" $N` (written in 001 step 2) | 001, 002 | exit 0 |
| 12 | `L same-tests "$S/0131-same-tests.sh"` (written in 001 step 2) | 001 | exit 0 |
| 13 | `L ipc sh -c 'bun run ipc:build && git diff --exit-code src/lib/ipc/bindings.ts'`, then `L bun-test bun test` | 002 | exit 0 each |
| 14 | the smoke run (001 step 9, 002 step 8) | 001, 002 | as the stage says |

Notes:

- Steps 4 and 6 are the only local checks of the Windows and Linux code. Windows tests run
  first in CI (`native.yml`, `windows-2025`), after the push.
- `cargo test --workspace` needs cmake, a C compiler and the V8 download. The first build takes
  minutes.
- `Cargo.lock` is expected to stay unchanged: tokio is already listed under `strudeland-ctl`.
  If it changes, the report shows the diff.
- **Each stage report lists:**
  - the exit line of every log;
  - the `server::` test names from steps 5 and 6;
  - the literal-count table (step 11);
  - the smoke stderr lines;
  - any pin that is platform-gated, and why.

## STOP, and report to the user, on any of these

Do not work around them:

- any change to a detail text, a stderr line, a status string, a socket or lock path, a file
  mode, or the wire;
- a pin or moved test that needs a changed expected literal, composition or assertion to
  pass, or a moved test body that is not byte-identical;
- a literal count different from the expected table;
- a needed edit to a frozen file;
- in 002, a needed change to any ctl file (the API is fixed in 001);
- a CI failure on Windows or Ubuntu in a `server::` or `control::` test whose fix would
  change an asserted text or behaviour;
- the smoke stderr lines differing between 001 and 002.

Never loosen a check, and never re-record an expectation, to absorb a difference.

## Commits

- Use plain Conventional Commits, with the message each stage file gives and no task or plan
  identifiers. End each one with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed.
