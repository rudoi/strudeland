# 0128 implementation entry point

- **Branch:** `llm/andrew/strudeland/0128-app-prefs`.
- **Plan:** [the proposal](../0128-app-prefs.md). It is the contract: `AppData`, `Loaded<T>`,
  the bad-file policy table, the documents, the limiter wiring and the migration table, plus the
  rejected options and the decisions.
- **Base:** `8c13c8c`. Diff against it, never against a local `main`, which is stale.
- **Evidence:** [reverify.md](reverify.md), with file and line at `8c13c8c`. Read its store
  table, "User decisions" and "Data safety" before you edit anything.
- **Background reading:**
  - `AGENTS.md`: the `strudeland-project`, voice memo and prefs bullets, and the IPC rule;
  - `crates/strudeland-project/src/{prefs,recents,instruments}.rs`;
  - `crates/strudeland-record/src/takes/{prefs,mod,testing}.rs`;
  - `src-tauri/src/{project,recording,lib}.rs`;
  - `src/lib/prefs/`, `src/lib/app.ts`.

  `GLOSSARY.md` gets no edit.

## Stages

The stages run in order. Each one is one commit that is green on every gate below.

1. [001-app-data.md](001-app-data.md): `AppData`, `Loaded<T>` and the document machinery.
   `Prefs` and `Recents` move onto it, `InstrumentLibrary::open_default` builds on it, and the
   app holds it as managed state. No byte changes.
2. [002-recording-input.md](002-recording-input.md): `RecordingInput` (`recording.json`) moves
   into `strudeland-project` on the same machinery, and `Takes` takes `&AppData`. No byte changes.
3. [003-limiter.md](003-limiter.md): `limiter` in `Prefs`, the store and the app wiring, the
   one-time `localStorage` migration, and `limiter-pref.ts` deleted. This is the only stage with
   visible changes, and it makes only the ones the proposal lists.

There is no docs stage. Each stage updates the `AGENTS.md` lines it makes stale (proposal,
Decisions).

The stages are sequential: 002 uses 001's machinery, and 003 adds a field to the 001 `Prefs`
byte test.

## Rules for every stage

- **No visible change beyond the proposal's list.** The following stay byte-identical:
  - every command result, error kind and message (except the no-data-dir message the proposal
    names);
  - every event;
  - `src/lib/ipc/bindings.ts`;
  - `recent.json` and `recording.json` for every value;
  - `prefs.json` until stage 003 adds `limiter`;
  - the control socket's `state`.

  If a stage seems to need a different output, stop and report it.
- **The recording transcript is frozen.** In `src-tauri/src/recording/transcript.rs`, the region
  between `// --- frozen: begin ---` and `// --- frozen: end ---` and the region between
  `// --- expected: begin ---` and `// --- expected: end ---` stay byte-identical to `8c13c8c`.
  Only the adapters above the frozen region may change, and only in stage 002.
- **Replace tests, don't layer them.** A test whose subject moved is deleted where it was and
  rewritten with the same assertions against the new owner. Each stage file lists what is
  deleted, what moves and what stays.
- **New logic ships with tests** in the same commit. The stage files name them.
- **Don't touch** `crates/strudeland-cli/`, `crates/strudeland-ctl/`, `crates/strudeland-live/`,
  `src/lib/ipc/bindings.ts` (only `ipc:build` may write it, and here it must write nothing) or
  `src/lib/control/`. Each stage file adds its own untouched paths.
- **Test cadence.** While working, run only the targeted tests the stage file names. When the
  stage is done, run each gate once, log it, and read the log rather than re-running it. `S` is
  your session's scratchpad directory and `N` the stage number (`001`, `002`, `003`).

  ```sh
  cargo fmt --all
  cargo test --locked -p strudeland-project -p strudeland-record -p strudeland-app > "$S/0128-$N-crates.log" 2>&1; echo "exit $?" >> "$S/0128-$N-crates.log"
  cargo test --locked --workspace > "$S/0128-$N-cargo-test.log" 2>&1; echo "exit $?" >> "$S/0128-$N-cargo-test.log"
  cargo clippy --locked --workspace --all-targets -- -D warnings > "$S/0128-$N-clippy.log" 2>&1; echo "exit $?" >> "$S/0128-$N-clippy.log"
  cargo fmt --all --check > "$S/0128-$N-fmt.log" 2>&1; echo "exit $?" >> "$S/0128-$N-fmt.log"
  bun run format
  bun test > "$S/0128-$N-bun.log" 2>&1; echo "exit $?" >> "$S/0128-$N-bun.log"
  bun run check > "$S/0128-$N-check.log" 2>&1; echo "exit $?" >> "$S/0128-$N-check.log"
  bun run lint > "$S/0128-$N-lint.log" 2>&1; echo "exit $?" >> "$S/0128-$N-lint.log"
  # The IPC contract: the generator writes nothing.
  bun run ipc:build > "$S/0128-$N-ipc.log" 2>&1; echo "exit $?" >> "$S/0128-$N-ipc.log"
  git diff --exit-code -- src/lib/ipc/bindings.ts >> "$S/0128-$N-ipc.log" 2>&1; echo "exit $?" >> "$S/0128-$N-ipc.log"
  # The frozen transcript regions.
  expected() { sed -n '/\/\/ --- expected: begin ---/,/\/\/ --- expected: end ---/p'; }
  frozen() { sed -n '/\/\/ --- frozen: begin ---/,/\/\/ --- frozen: end ---/p'; }
  T=src-tauri/src/recording/transcript.rs
  { diff <(git show "8c13c8c:$T" | expected) <(expected < "$T"); echo "exit $?"; \
    diff <(git show "8c13c8c:$T" | frozen) <(frozen < "$T"); echo "exit $?"; } > "$S/0128-$N-transcript.log" 2>&1
  # Paths no stage touches.
  git diff --stat 8c13c8c -- crates/strudeland-cli crates/strudeland-ctl crates/strudeland-live \
    src/lib/ipc src/lib/control > "$S/0128-$N-untouched.log"; echo "exit $?" >> "$S/0128-$N-untouched.log"
  ```

  - Every log ends `exit 0`, and the ipc and transcript logs have every `exit` line at 0 with no
    diff lines.
  - `$(git show 8c13c8c:$T | expected | wc -l)` is over 100 and the same count for `frozen` is
    over 50, so an empty extraction cannot pass.
  - `untouched.log` lists no files, and neither does the stage's own untouched check.
  - `bun run format` may rewrite files. Stage them, then run `bun run lint`.
  - `cargo test --workspace` needs `cmake`, a C compiler and the V8 download the host brings.
    The first build takes minutes.
- **Code behind `cfg`.** The gates run on macOS, so clippy there never compiles the
  `cfg(not(any(target_os = "linux", target_os = "macos")))` branch of `create_directory`, which
  `open_in` reuses. Reason it through. If `rustup target list --installed` shows
  `x86_64-pc-windows-msvc`, also run
  `cargo clippy --locked -p strudeland-project --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  and log it. Rely on the PR's Windows and Linux CI otherwise.
- **STOP and report** without committing if any of these happen:
  - a saved file's bytes would change for any value. The one exception is stage 003's
    `"limiter"` key in `prefs.json`;
  - a file that loads today would load a different value, or one that falls back today would
    load;
  - any value would be lost: a recents entry, a pref, the saved input, the `localStorage`
    limiter before a confirmed save;
  - `bindings.ts` would change, or a command's name, argument key, result shape or error kind
    would change;
  - the transcript's frozen region or expected block would have to change;
  - any other visible change: UI copy, a notice, an event, a CLI output or exit code.
- **Commits are plain Conventional Commits** with the message the stage gives, no task or plan
  identifiers, ending with:

  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  ```

- **No scratch worktrees** (`git worktree add`), and no bare `git stash`. To set work aside, make
  a WIP commit. Nothing under `$S` is committed.
- **Manual smoke (the PR's test plan).** The user runs these on `bun run app:signed`. Never start
  playback or a recording for them.
  1. With the limiter on from a build before this change (`localStorage` `strudeland.limiter` =
     `'1'`), launch the new build. The limiter shows on. `prefs.json` has `"limiter": true`, and
     the key is gone from the webview's storage.
  2. Toggle the limiter off, quit and relaunch. It stays off, and `prefs.json` says `false`.
  3. Arm a non-default input device or channel, quit and relaunch. The picker shows the same
     device and channel, and `recording.json` reads as before.
  4. Open a recent project from the welcome screen. The list matches the one before the upgrade.
