# 001: the app lifecycle module

## Goal

Move the reload and exit hooks' calls out of `src-tauri/src/lib.rs` into
`src-tauri/src/lifecycle.rs` (`on_reload`, `on_exit`). Keep the same calls in the same order,
document the order and Decisions 1 to 4, and test coverage and order over the real states.
Nothing observable changes (see [000-prompt.md](000-prompt.md), Rules).

## Prerequisites

- The tree is code-identical to `00feb37`: `git diff --name-only 00feb37 HEAD` lists only
  `plans/`.
- The stable toolchain builds the workspace, and `bun install` has run (for `ipc:build`).

## Files

- Edited: `AGENTS.md`, `src-tauri/src/lib.rs`, `src-tauri/src/control.rs`,
  `src-tauri/src/app_quit.rs`.
- New: `src-tauri/src/lifecycle.rs`.
- New, not committed: `$S/0135-literals.tsv`, `$S/0135-literals.sh` and `$S/0135-hooks.sh`.
  The session that wrote this plan already wrote them and red-ran them on the unchanged tree.
  Their full text is in the appendix. Recreate them from it when `$S` differs (tabs separate the
  `.tsv` columns), then run `chmod +x "$S"/0135-*.sh`.

## Steps

1. **Baseline, before any edit.** With `S` and `L` from 000-prompt.md:
   - `L literals-base "$S/0135-literals.sh"`. Exit 1, with `FAIL` on exactly the 5 rows whose
     `base` and `after` differ: the two moved comments, once in `lib.rs` and once in
     `lifecycle.rs`, and the deleted `shutdown` doc line. The other 9 rows and the
     `eprintln!` total (20) pass.
   - `L hooks-base "$S/0135-hooks.sh"`. Exit 1, with `FAIL` on every check except
     `transcript.rs unchanged`. The no-own-call check reads `got 7`. `lifecycle.rs` does not
     exist yet, so its seven call counts read `got 0` and its two call-order checks read
     `got ` (empty).

   Any other result on the unchanged tree means that the scripts or the tree differ from the
   plan's: STOP. These red runs show that the scripts detect the change they guard.

2. **`control.rs`.**
   - `:169` `fn close_server(&self)` → `pub fn close_server(&self)`. Its doc (`:167-168`)
     stays word for word.
   - Delete `:562-566`: the two doc lines `/// Closes the endpoint on exit …` and
     `/// accept task.`, `pub fn shutdown(app: &AppHandle) { … }`, and the blank line after it.
   - `:26` `use tauri::{AppHandle, Emitter, Manager, State};` →
     `use tauri::{AppHandle, Emitter, State};`. Its only use of `Manager` was `:565`. `AppHandle`
     is still used by `start` and `TauriEmitter`.
   - Directly after the closing brace of the `impl Bridge { … }` block that starts at `:144`
     (the one holding `new`, `detach`, `reply` and `call`), add:

     ```rust
     #[cfg(test)]
     impl Bridge {
         /// Holds an idle server, as `start` leaves a listening bridge.
         pub(crate) fn serve_idle(&self) {
             self.set_server(Server {
                 endpoint: Weak::new(),
                 task: tauri::async_runtime::spawn(std::future::pending::<()>()),
             });
         }

         /// Whether a server is held (`close_server` takes it).
         pub(crate) fn serving(&self) -> bool {
             lock(&self.server).is_some()
         }
     }
     ```

     `Weak` is already imported (`:16`). `tauri::async_runtime::spawn` in a plain `#[test]` is
     what `start_path_serves_and_closes_a_temp_endpoint` already does through `start_on`.

3. **`app_quit.rs`.** After `impl QuitRequest { … }` (`:74-83`), add:

   ```rust
   #[cfg(test)]
   impl QuitState {
       /// Ready with request 0 pending: the webview is asking the user.
       pub(crate) fn asking(&self) {
           self.set_ready(true);
           self.request(|_| Ok(()));
       }

       /// Ready with exit approved: the export permit is held.
       pub(crate) fn approved(&self, exports: &ExportState) {
           self.asking();
           assert!(self.reply(0, true, exports).unwrap());
       }

       /// `(ready, pending, holds the exit permit)`.
       pub(crate) fn view(&self) -> (bool, Option<u32>, bool) {
           let state = self.lock();
           (state.ready, state.pending, state.exit_permit.is_some())
       }
   }
   ```

   `request` is already `#[cfg(any(target_os = "macos", test))]`, and so is `next_id`. Nothing
   else in the file changes.

4. **`lifecycle.rs` (new).** The functions, with the step log as the order seam. Copy the two
   exit comments verbatim from `lib.rs:115` and `:119`. Neither text may appear a second time in
   the file: the literal gate counts each exactly once. Paraphrase them in the module docs.

   ```rust
   //! What a reload of the main webview and the app exit stop, and in what order.
   //!
   //! `lib.rs` only filters the events (the `main` webview's `PageLoadEvent::Started`, which also
   //! fires on the first load, when every call is a no-op; and `RunEvent::Exit`) and looks the
   //! states up. A recorder or playback state not managed yet is `None` and skipped.
   //!
   //! [`on_reload`], in this order:
   //! 1. [`QuitState::detach`]: a pending Quit request belonged to the old page. An approved
   //!    exit keeps its export permit.
   //! 2. [`Bridge::detach`]: pending control requests fail with `page reloaded`, and later ones
   //!    answer `not ready` until the new page attaches. The socket stays open.
   //! 3. [`Takes::on_reload`]: a take in progress is finalized with the fallback sidecar and the
   //!    recorder is disarmed, without events.
   //! 4. [`playback::end_session`]: the webview that owned the session is gone.
   //!
   //! Playback stops last here. The order predates this module and is kept: a recorder
   //! finalize can delay the stop by the WAV and sidecar write, and a take's song backing plays
   //! on superdough, not on this session.
   //!
   //! [`on_exit`], in this order: playback first, so the audio is the first thing to stop; then
   //! the recorder, ahead of the control socket, which could otherwise hold the audio up; then
   //! [`Bridge::close_server`] (on Unix it removes the socket, then releases the lock).
   //!
   //! Left alone on purpose:
   //! - A reload does not cancel an audio export. Exports have no cancel mechanism and run to
   //!   completion. Until one finishes, a new export answers `busy` and a Quit is refused.
   //! - The exit does not reset [`QuitState`] or release the export permit. An approved exit
   //!   holds the permit until the process ends, so no export can start between the approval
   //!   and the exit.
   //! - The project watcher, the piano roll host and the instrument preview. The project stays
   //!   open across a reload, a piano roll inspection is bounded, and the preview keeps no
   //!   native state. At exit, the end of the process stops them.

   #[cfg(test)]
   use std::cell::RefCell;

   use strudeland_record::takes::Takes;

   use crate::app_quit::QuitState;
   use crate::control::Bridge;
   use crate::playback::{self, PlaybackState};

   /// The main webview started a new page load (a reload or HMR).
   pub fn on_reload(
       quit: &QuitState,
       bridge: &Bridge,
       takes: Option<&Takes>,
       playback: Option<&PlaybackState>,
   ) {
       quit.detach();
       step("quit");
       bridge.detach();
       step("bridge");
       if let Some(takes) = takes {
           takes.on_reload();
           step("takes");
       }
       if let Some(state) = playback {
           playback::end_session(state);
           step("playback");
       }
   }

   /// The app exits (`RunEvent::Exit`).
   pub fn on_exit(playback: Option<&PlaybackState>, takes: Option<&Takes>, bridge: &Bridge) {
       // Audio stops first.
       if let Some(state) = playback {
           playback::end_session(state);
           step("playback");
       }
       // Before the control socket, so a slow socket cannot delay the audio.
       if let Some(takes) = takes {
           takes.shutdown();
           step("takes");
       }
       bridge.close_server();
       step("bridge");
   }

   #[cfg(test)]
   thread_local! {
       static STEPS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
   }

   /// Records that a call ran, so the tests can check the order. Does nothing in the app.
   #[cfg(test)]
   fn step(name: &'static str) {
       STEPS.with(|steps| steps.borrow_mut().push(name));
   }

   #[cfg(not(test))]
   fn step(_: &'static str) {}
   ```

   - The parameter named `playback` shadows the module `playback` only in the value
     namespace, so `playback::end_session` still resolves. If clippy or a reader objects,
     rename the parameter `session` in both functions; do not rename the module path.
   - Intra-doc links to private items (`Bridge::close_server` is now `pub`, the rest already
     are) must not warn under `cargo doc`. If one does, write it as plain backticks.
   - **Seam size.** Count the seam lines: the `thread_local!` block with its attribute (4),
     both `step` definitions with their attributes and doc (7), and the 7 `step(…)` calls. As
     sketched that is 18, after `cargo fmt`. The test-only `RefCell` import (2 lines) only keeps
     the `static` line within the width and is reported next to the count. More than 20 seam
     lines is a STOP: drop the order test (step 5, test 5), the log and the calls, and report
     it.

5. **Tests** (`lifecycle.rs`, `#[cfg(test)] mod tests`). They use only real states:
   - `QuitState::default()` and `ExportState::default()`;
   - `Bridge::new()`, with `serve_idle` for a listening bridge;
   - `PlaybackState::default()`, with a session started by `playback::start` over
     `strudeland_live::testing::FakeOutput` (`Pace::RealTime`, 48 kHz, as `playback.rs`'s
     `fake()`) and an `EventSink` that drops events;
   - `Takes` through `strudeland_record::testing::TakesRig::new(tmp.path(), project,
     Limits::default(), TakesConfig { app_version: "test".into() })`, with
     `project = Arc::new(Project::create(&tmp.path().join("alpha"), "alpha").unwrap())`, as
     `takes/tests.rs:28-40` builds it.

   A take in progress comes from `rig.take()`, which arms, starts `main-01` and records 1000
   frames. Each test is short and names what it pins:

   1. `a_reload_resets_every_state`: set up `quit.asking()`, a bridge that is `attach()`ed and
      `serve_idle()`d, a take in progress (`handle = rig.take()`), and a playback session `id`.
      Call `on_reload(&quit, &bridge, Some(&rig.takes), Some(&state))`. Then assert:
      - `quit.view() == (false, None, false)`;
      - `!bridge.attached()` and `bridge.serving()` (the socket stays open);
      - `!handle.is_open()`;
      - `rig.takes.status(&rig.app_data()).state == RecState::Idle`;
      - `rig.files() == ["main-01.json", "main-01.wav"]`;
      - `playback::stop(&state, id)` is `Err` with kind `CommandErrorKind::Stale`.

      A one-line comment notes that `Bridge::detach` failing pending requests is covered by
      `control.rs`'s existing `detach_fails_pending_requests_and_later_ones_are_not_ready`, so
      this test does not repeat it.
   2. `a_reload_keeps_an_approved_exit`: set up `quit.approved(&exports)`, then call
      `on_reload(&quit, &Bridge::new(), None, None)`. Assert that
      `quit.view() == (false, None, true)` and that `exports.reserve().is_err()`. This pins the
      reload half of Decision 2 (`QuitState::detach` keeps the permit).
   3. `the_exit_stops_audio_recorder_and_socket`: set up a playback session, a take in progress
      and a `serve_idle()`d bridge. Call `on_exit(Some(&state), Some(&rig.takes), &bridge)`.
      Then assert:
      - the stop is `Stale`;
      - `!handle.is_open()`, state `Idle`, and `rig.files()` as in test 1;
      - `!bridge.serving()`.

      `on_exit` takes no `QuitState` and no `ExportState`, so Decisions 1 and 2 hold by its
      signature. Say so in a one-line comment rather than asserting on states the hook cannot
      reach.
   4. `the_hooks_skip_states_not_managed_yet`: `on_reload(&QuitState::default(), &Bridge::new(),
      None, None)` and `on_exit(None, None, &Bridge::new())` do not panic. They record
      `["quit", "bridge"]` and `["bridge"]` (read with `STEPS.take()` after clearing it first).
   5. `the_hooks_run_in_the_documented_order`: clear `STEPS`, then call `on_reload` with all
      four states (idle ones are enough), and assert
      `STEPS.take() == ["quit", "bridge", "takes", "playback"]`. Then call `on_exit` with all
      three, and assert `["playback", "takes", "bridge"]`.

   Use the imports the tests need (`tempfile`, `strudeland_project::Project`,
   `strudeland_record::{testing::{Limits, TakesRig}, takes::{RecState, TakesConfig}}`,
   `strudeland_live::{Event, Output, OutputError, OutputInfo, testing::{FakeOutput, Pace}}`,
   `crate::playback::EventSink`, `crate::command_error::CommandErrorKind`,
   `crate::export_audio::ExportState`). Adjust a path only if the compiler names the right one;
   do not add a re-export to another crate.

6. **`lib.rs`.**
   - Add `mod lifecycle;` between `mod instruments;` and `mod piano_roll;` (alphabetical, as
     the list is).
   - The comment at `:37` stays above `.on_page_load`. The closure becomes:

     ```rust
     .on_page_load(|webview, payload| {
         if webview.label() == "main" && matches!(payload.event(), PageLoadEvent::Started) {
             let recorder = webview.try_state::<recording::RecorderState>();
             let session = webview.try_state::<playback::PlaybackState>();
             lifecycle::on_reload(
                 &webview.state::<app_quit::QuitState>(),
                 &webview.state::<Arc<control::Bridge>>(),
                 recorder.as_ref().map(|r| &*r.0),
                 session.as_deref(),
             );
         }
     })
     ```

   - The `.run` closure becomes:

     ```rust
     .run(|app, event| {
         if let tauri::RunEvent::Exit = event {
             let session = app.try_state::<playback::PlaybackState>();
             let recorder = app.try_state::<recording::RecorderState>();
             lifecycle::on_exit(
                 session.as_deref(),
                 recorder.as_ref().map(|r| &*r.0),
                 &app.state::<Arc<control::Bridge>>(),
             );
         }
     });
     ```

   - `use tauri::Manager;` stays (`state`, `try_state` and `manage` need it).

7. **`AGENTS.md`.** In the `src-tauri/` bullet (`:143-146`), after
   `` `watcher.rs` (`project://changed` events), `` insert
   `` `lifecycle.rs` (what a main-webview reload and the app exit stop, in order, and what they
   leave alone), ``. Change nothing else.

8. `cargo fmt --all`, then run the full gate (000-prompt.md) once, in order.

## Validation

**Call order.** Read the new functions next to `00feb37`'s `lib.rs:38-49` and `:113-125`, and
put this table in the stage report with each row checked:

| Hook | # | `00feb37` `lib.rs` | `lifecycle.rs` |
|---|---|---|---|
| reload | 1 | `:40` `QuitState::detach` | `quit.detach()` |
| reload | 2 | `:41` `Bridge::detach` | `bridge.detach()` |
| reload | 3 | `:42-44` `if let Some(recorder) = try_state::<RecorderState>` → `recorder.0.on_reload()` | `if let Some(takes)` → `takes.on_reload()` |
| reload | 4 | `:45-47` `if let Some(p) = try_state::<PlaybackState>` → `playback::end_session(&p)` | `if let Some(state)` → `playback::end_session(state)` |
| exit | 1 | `:116-118` `playback::end_session` (skipped if absent) | the same |
| exit | 2 | `:120-122` `recorder.0.shutdown()` (skipped if absent) | `takes.shutdown()` (the same) |
| exit | 3 | `:123` `control::shutdown(app)` → `close_server` | `bridge.close_server()` |

Also check these:

- The filter (`label == "main"`, `PageLoadEvent::Started`, `RunEvent::Exit`) is unchanged.
- The only new statements in `lib.rs` are state lookups: no side effects, and no call moves.
- Test 5 passes, and gates 6 and 7 pass (gate 6 also counts each call in `lifecycle.rs` and
  checks their line order per function).

## Commit message

```
refactor(app): run the reload and exit hooks from one lifecycle module

lib.rs keeps the event filters and state lookups; lifecycle.rs runs the calls in the same order
and documents why, and what each hook leaves alone. Tests cover every state each hook resets and
the call order over the real states. control::shutdown is gone; close_server is called directly.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```

## Appendix: gate scripts

`$S/0135-literals.tsv`. The columns are literal, path, base count and after count, separated by
tabs:

```
Audio stops first.	src-tauri/src/lib.rs	1	0
Audio stops first.	src-tauri/src/lifecycle.rs	0	1
Before the control socket, so a slow socket cannot delay the audio.	src-tauri/src/lib.rs	1	0
Before the control socket, so a slow socket cannot delay the audio.	src-tauri/src/lifecycle.rs	0	1
A reload or HMR of the main webview: requests answer `not ready` until it attaches again.	src-tauri/src/lib.rs	1	1
/// Closes the endpoint on exit (Unix: removes the socket, then releases the lock) and stops the	src-tauri/src/control.rs	1	0
/// Closes the endpoint (Unix: removes the socket, then releases the lock) and stops the	src-tauri/src/control.rs	1	1
recording: the event forwarder panicked	crates/strudeland-record/src/takes/mod.rs	1	1
control: listening on	src-tauri/src/control.rs	1	1
control: disabled	src-tauri/src/control.rs	2	2
could not deliver application Quit request	src-tauri/src/app_quit.rs	1	1
Ends any session: on a page reload (the webview that owned it is gone) and at the app exit.	src-tauri/src/playback.rs	1	1
eprintln!	src-tauri/src/lifecycle.rs	0	0
eprintln!	src-tauri/src/lib.rs	0	0
```

`$S/0135-literals.sh`:

```sh
#!/bin/sh
# Literal counts for 0135: each row is literal<TAB>path<TAB>base<TAB>after. Prints the count now
# and PASS when it equals `after`; exits 1 on any FAIL. Run from the repo root. A missing file
# counts 0. The total eprintln! count over src-tauri/src and strudeland-record/src must stay 20.
S=$(dirname "$0")
fail=0
while IFS="$(printf '\t')" read -r lit path base after; do
  if [ -f "$path" ]; then now=$(grep -cF -- "$lit" "$path"); else now=0; fi
  if [ "$now" = "$after" ]; then r=PASS; else r=FAIL; fail=1; fi
  printf '%s\t%s\tbase %s\tafter %s\tnow %s\t%s\n' "$r" "$path" "$base" "$after" "$now" "$lit"
done < "$S/0135-literals.tsv"
total=$(grep -rhoF 'eprintln!' src-tauri/src crates/strudeland-record/src | wc -l | tr -d ' ')
if [ "$total" = 20 ]; then r=PASS; else r=FAIL; fail=1; fi
printf '%s\teprintln! total\tbase 20\tafter 20\tnow %s\n' "$r" "$total"
exit $fail
```

`$S/0135-hooks.sh`:

```sh
#!/bin/sh
# 0135: lib.rs's reload and exit hooks call only lifecycle; the moved calls live only there.
# Prints one PASS/FAIL line per check; exits 1 on any FAIL. Run from the repo root.
fail=0
check() { if [ "$2" = "$3" ]; then echo "PASS $1 ($2)"; else echo "FAIL $1 (got $2, want $3)"; fail=1; fi; }
lib=src-tauri/src/lib.rs
check "lib.rs declares mod lifecycle" "$(grep -c '^mod lifecycle;$' $lib)" 1
check "lib.rs calls lifecycle::on_reload once" "$(grep -c 'lifecycle::on_reload(' $lib)" 1
check "lib.rs calls lifecycle::on_exit once" "$(grep -c 'lifecycle::on_exit(' $lib)" 1
check "lib.rs makes no hook call of its own" \
  "$(grep -cE '\.detach\(|\.on_reload\(|end_session|\.shutdown\(|close_server|control::shutdown' $lib)" 0
check "control::shutdown is gone" "$(grep -c '^pub fn shutdown(' src-tauri/src/control.rs)" 0
check "end_session callers: playback.rs and lifecycle.rs" \
  "$(grep -rlF 'end_session(' src-tauri/src | sort | tr '\n' ' ')" \
  "src-tauri/src/lifecycle.rs src-tauri/src/playback.rs "
check ".on_reload()/.shutdown() callers: lifecycle.rs and the transcript" \
  "$(grep -rlE '\.on_reload\(\)|\.shutdown\(\)' src-tauri/src | sort | tr '\n' ' ')" \
  "src-tauri/src/lifecycle.rs src-tauri/src/recording/transcript.rs "
check ".close_server() callers: control.rs and lifecycle.rs" \
  "$(grep -rlF '.close_server()' src-tauri/src | sort | tr '\n' ' ')" \
  "src-tauri/src/control.rs src-tauri/src/lifecycle.rs "
life=src-tauri/src/lifecycle.rs
cnt() { if [ -f "$life" ]; then grep -cF -- "$1" "$life"; else echo 0; fi; }
check "lifecycle.rs: quit.detach() once" "$(cnt 'quit.detach()')" 1
check "lifecycle.rs: bridge.detach() once" "$(cnt 'bridge.detach()')" 1
check "lifecycle.rs: takes.on_reload() once" "$(cnt 'takes.on_reload()')" 1
check "lifecycle.rs: end_session( twice" "$(cnt 'end_session(')" 2
check "lifecycle.rs: takes.shutdown() once" "$(cnt 'takes.shutdown()')" 1
check "lifecycle.rs: bridge.close_server() once" "$(cnt 'bridge.close_server()')" 1
check "lifecycle.rs: step(\" seven times" "$(cnt 'step("')" 7
# The calls of one function, in line order: from `pub fn NAME` to the next `}` in column 0.
calls() {
  [ -f "$life" ] || return 0
  awk -v f="pub fn $1(" 'index($0, f) == 1 { on = 1; next } on && /^}/ { exit } on' "$life" |
    grep -oE 'quit\.detach\(\)|bridge\.detach\(\)|takes\.on_reload\(\)|end_session\(|takes\.shutdown\(\)|bridge\.close_server\(\)|step\("[a-z]+"\)' |
    tr '\n' ' '
}
check "on_reload call order" "$(calls on_reload)" \
  'quit.detach() step("quit") bridge.detach() step("bridge") takes.on_reload() step("takes") end_session( step("playback") '
check "on_exit call order" "$(calls on_exit)" \
  'end_session( step("playback") takes.shutdown() step("takes") bridge.close_server() step("bridge") '
check "transcript.rs unchanged" \
  "$(shasum -a 256 src-tauri/src/recording/transcript.rs | cut -d' ' -f1)" \
  230c83361d7a29b16e7abcff81576a1cfa8fbcfe4dfbaf717dc31fbca88a2e81
exit $fail
```
