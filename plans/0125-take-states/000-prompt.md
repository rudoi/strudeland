# 0125 implementation entry point

- **Branch:** `llm/andrew/strudeland/0125-take-states`.
- **Plan:** [the proposal](proposal.md). It is the contract: the state enum,
  `leave`, `Takes` and its ports, `TakeError`, the module layout, the rejected options and the
  decisions.
- **Base:** `d38b30e`. Diff against it, never against a local `main`: it is stale.
- **Evidence:** [reverify.md](reverify.md), with file and line at `d38b30e`. Read its event
  table, "Unchanged", "Quirks kept as they are" and the threading rules before you edit
  anything.
- **Background reading:** `AGENTS.md`, the voice memo rules and layout bullet;
  `crates/strudeland-record/README.md`; `GLOSSARY.md`, **Take**. `GLOSSARY.md` gets no edit:
  `Takes` is the name of the type that owns the lifecycle, not a second term beside **Take**
  (one recording), and its doc comment says so.

## Stages

The stages run in order, and each one is one commit that is green with every gate below.

1. [001-transcript.md](proposal.md): the shared test rig file and the characterization
   transcript of every take flow. No production code changes.
2. [002-state-enum.md](proposal.md): `TakeState`, `leave`, `settle_ended`,
   `saved_channel` and `pending_loss_stem`, in place in `recording.rs`. The transcript is
   unchanged.
3. [003-takes-module.md](proposal.md): the move to `strudeland-record::takes` with
   `TakeError`, `TakeEvent`, the injected `app_version`, the fakes in `testing`, the test
   move, and the docs.

The stages are sequential: 002 needs 001's transcript as its guard, and 003 moves 002's code.

**CI gate between 001 and 002.** The local gates run on macOS only. After the 001 commit, push
the branch and open a draft PR against `main` (`gh pr create --draft`): `native.yml` and
`macos.yml` run only on `pull_request` and on pushes to `main`, never on a plain branch push.
Stage 002 starts only once both workflows are green on the 001 head (Windows x64, Ubuntu 24.04,
macOS). Later stages push to the same PR. Never force-push; a fix is a new commit.

## Rules for every stage

- **Zero visible change.** The following all stay byte-identical:
  - every `recording://state` payload and its order;
  - `recording://level`;
  - every command result, error kind and message;
  - sidecar contents, including `app.version`;
  - file outcomes;
  - threading behaviour.

  If a stage seems to need a different output, stop and report it.
- **The transcript is frozen after stage 001.** Two regions of
  `src-tauri/src/recording/transcript.rs` may not change:
  - the expected JSON, between `// --- expected: begin ---` and `// --- expected: end ---`;
  - `normalize`, the restricted-text literals, the flow functions and the test function,
    between `// --- frozen: begin ---` and `// --- frozen: end ---`.

  Only the call adapters above the frozen region may change, and only in stage 003. Stage 002
  does not edit `transcript.rs` at all. Never edit either region to absorb a difference.
- **Keep every quirk** that reverify lists (#155), even where the new shape makes a fix
  tempting.
- **Keep every threading rule:**
  - the forwarder holds a `Weak`;
  - the forwarder is joined outside the lock;
  - levels go out without the lock;
  - `ensure_permission` runs outside the lock behind `requesting`;
  - the stale-`arm_id` check and the `take_if` path match stay;
  - poison recovery stays;
  - the lock is held across arm, start and stop.
- **Don't touch** `src/`, `src/lib/ipc/bindings.ts`, `crates/strudeland-project/`,
  `crates/strudeland-record/src/{recorder,writer,source,pipeline,device,anchors,clock,levels,error}.rs`,
  or `crates/strudeland-cli/`. Don't run `ipc:build`. No command, event or kind changes.
- **Replace tests, don't layer them.** A test of moved code moves with the same assertions.
  Each stage file lists what is deleted, what moves and what stays.
- **New logic ships with tests** in the same commit. The stage files name them.
- **Test cadence.**
  - While working, run targeted tests:
    - `cargo test -p strudeland-app --lib recording::`;
    - from stage 003, also `cargo test -p strudeland-record --lib takes::`.
  - When the stage is done, run each gate once, log it, and read the log rather than re-running
    it. `S` is your session's scratchpad directory, and `N` is the stage number (`001`, `002`,
    `003`).
  - `T0` is the last stage-001 commit, the one CI went green on: the last commit that touched
    the transcript. Stage 002 never edits it, and stage 003 runs the gate before committing, so
    this still resolves to stage 001's head in both:
    `T0=$(git log -1 --format=%H -- src-tauri/src/recording/transcript.rs)`.

    ```sh
    cargo fmt --all
    cargo test --locked -p strudeland-record -p strudeland-app > "$S/0125-$N-crates.log" 2>&1; echo "exit $?" >> "$S/0125-$N-crates.log"
    cargo test --locked --workspace > "$S/0125-$N-cargo-test.log" 2>&1; echo "exit $?" >> "$S/0125-$N-cargo-test.log"
    cargo clippy --locked --workspace --all-targets -- -D warnings > "$S/0125-$N-clippy.log" 2>&1; echo "exit $?" >> "$S/0125-$N-clippy.log"
    cargo fmt --all --check > "$S/0125-$N-fmt.log" 2>&1; echo "exit $?" >> "$S/0125-$N-fmt.log"
    bun test > "$S/0125-$N-bun.log" 2>&1; echo "exit $?" >> "$S/0125-$N-bun.log"
    # From stage 002 on: the two frozen regions.
    expected() { sed -n '/\/\/ --- expected: begin ---/,/\/\/ --- expected: end ---/p'; }
    frozen() { sed -n '/\/\/ --- frozen: begin ---/,/\/\/ --- frozen: end ---/p'; }
    diff <(git show "$T0:src-tauri/src/recording/transcript.rs" | expected) \
         <(expected < src-tauri/src/recording/transcript.rs) > "$S/0125-$N-transcript.log" 2>&1; echo "exit $?" >> "$S/0125-$N-transcript.log"
    diff <(git show "$T0:src-tauri/src/recording/transcript.rs" | frozen) \
         <(frozen < src-tauri/src/recording/transcript.rs) > "$S/0125-$N-frozen.log" 2>&1; echo "exit $?" >> "$S/0125-$N-frozen.log"
    git diff --stat d38b30e -- src/ crates/strudeland-project crates/strudeland-cli crates/strudeland-live \
      crates/strudeland-record/src/{recorder,writer,source,pipeline,device,anchors,clock,levels,error}.rs \
      > "$S/0125-$N-untouched.log"; echo "exit $?" >> "$S/0125-$N-untouched.log"
    ```

  - Every log ends `exit 0`. The transcript log also has no diff lines, and
    `$(git show "$T0:…" | expected | wc -l)` is over 100, and the same count for `frozen` is
    over 50, so an empty extraction cannot pass. The frozen log has no diff lines either.
    `untouched.log` lists no files. `bun test` covers the IPC bindings staleness test.
  - `cargo test --workspace` needs `cmake`, a C compiler and the V8 download the host brings. The
    first build takes minutes.
  - **Code behind `cfg`.** The gates run on macOS, so clippy there never compiles
    `cfg(windows)`, `cfg(target_os = "linux")` or `cfg(not(target_os = "macos"))` code. Each
    stage file names the code behind `cfg` it touches; reason each one through. If
    `rustup target list --installed` shows `x86_64-pc-windows-msvc`, also run
    `cargo clippy --locked -p strudeland-record --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
    and log it. If that fails for reasons outside the change (a C dependency, the linker), note
    it in the log and rely on the PR's Windows and Linux CI.
- **STOP and report** without committing if any of these happen:
  - the transcript's expected JSON would have to change;
  - any wire text or error kind or message would change;
  - a test needs a new wait or retry to pass, which means a new interleaving;
  - a command's signature or `bindings.ts` would change;
  - a quirk would disappear.
- **Commits are plain Conventional Commits** with the message the stage gives, no task or plan
  identifiers, ending with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside,
  make a WIP commit. Nothing under `$S` is committed.
- **Follow-ups already filed:** #155 (the kept quirks). Cite it in the PR body; file anything
  new with `keitora task new`.
- **Manual smoke (the PR's test plan).** The user runs these on `bun run app:signed`. Never start
  playback or a recording for them.
  1. Arm the input, record a memo with the song backing, stop: the memo lists with its length,
     plays, and its `.json` has a bar-1 grid.
  2. Record past the length cap. This needs a debug cap, or skip it and rely on the transcript's
     cap flow. The take stops by itself with the limit notice, and a later stop keeps the memo.
  3. Unplug a USB input mid-take: the device-lost notice appears, the memo is kept, and
     re-arming works.
  4. Deny microphone access (System Settings, Privacy, Microphone), then arm: the permission
     notice appears and no stream opens. Grant it again and arm.
  5. Reload the webview mid-take (Cmd-R in `tauri dev`): the memo is saved, and the recorder is
     idle after the reload.
