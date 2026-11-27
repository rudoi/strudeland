# 0135 app lifecycle: step 0 re-verification

Checked on `llm/andrew/strudeland/0135-app-lifecycle` at `00feb37` (origin/main). Read only, and
the app was not run.

## 1. Lifecycle hooks today

The claim's line numbers have moved. The hooks are now at `src-tauri/src/lib.rs:38-49` and
`:113-125`. Since 63b945d only names and line positions have changed: #40 turned
`recording::on_reload`/`shutdown` into `Takes::on_reload`/`shutdown` methods, and #43 and the
piano roll added managed states. Neither hook changed what it does.

**Reload** (`on_page_load`, `lib.rs:38-49`): it runs only for the `main` label and
`PageLoadEvent::Started`. It also fires on the first load, when every call is a no-op.

| # | Call | What it does |
|---|---|---|
| 1 | `QuitState::detach` `lib.rs:40` → `app_quit.rs:30-38` | `ready = false`, `pending = None`. It leaves `exit_permit` alone. |
| 2 | `Bridge::detach` `lib.rs:41` → `control.rs:193-196` | `attached = false`, then fails every pending request with `page_reloaded`. The socket stays open. |
| 3 | `Takes::on_reload` (`try_state`) `lib.rs:42-44` → `crates/strudeland-record/src/takes/mod.rs:810-820` | Idle is a no-op. Otherwise it finalizes a running take with the fallback sidecar, disarms, and goes idle with no events. |
| 4 | `playback::end_session` (`try_state`) `lib.rs:45-47` → `playback.rs:188-190` | `state.end(None)`: stops any session. Idempotent. |

**Exit** (`RunEvent::Exit`, `lib.rs:113-125`):

| # | Call | Comment in code |
|---|---|---|
| 1 | `playback::end_session` `lib.rs:116-118` | "Audio stops first." |
| 2 | `Takes::shutdown` `lib.rs:120-122` → `takes/mod.rs:824-842` (finalize, idle, shut down and join the recorder and forwarder outside the lock; on a forwarder panic it prints `recording: the event forwarder panicked` to stderr) | "Before the control socket, so a slow socket cannot delay the audio." |
| 3 | `control::shutdown(app)` `lib.rs:123` → `control.rs:564-566` → `Bridge::close_server` `control.rs:169-177` (closes the endpoint, which on Unix removes the socket and releases the lock, then aborts the accept task) | (none) |

**Other hooks:**
- The app has no `on_window_event`, `CloseRequested`, `ExitRequested` or `Destroyed` handler.
  The only matches for these names in `src-tauri/src` and `crates/*/src` are in `lib.rs`.
  Window close is guarded in the frontend (`src/lib/shell.ts:247-248` `canCloseWindow`, through
  `onCloseRequested` in `src/lib/project/api.ts:123,244`). When the last window closes, Tauri
  exits and runs `RunEvent::Exit`.
- The macOS Quit flow (`app_quit.rs:85-123`, `macos::should_terminate` `:175-193`) ends in
  `app.exit(0)` (`:103`, `:120`), so it also reaches `RunEvent::Exit`. A system quit is answered
  with `TerminateNow` (`:180-186`). That path is assumed to reach `RunEvent::Exit` through Tao's
  `applicationWillTerminate`. I did not verify this, because doing so means running the app.
- Not in either hook:
  - The project watcher (`watcher.rs:406-480`, which stops on drop and is owned by
    `ProjectState`) and `PianoRollState` (`piano_roll.rs:19`: a cached `Host` behind a 2 s
    inspection timeout) are not touched.
  - Instrument preview keeps no native state. `instrument_preview` returns WAV bytes and the
    webview plays them.
  - `ExportState` (`export_audio.rs:25-35`) is not touched.

## 2. Asymmetries

| Asymmetry | Classification | Evidence |
|---|---|---|
| Reload does not touch export | **Intended (no mechanism to cancel)** | An export has no cancel path. The worker holds the permit on `spawn_blocking` (`export_audio.rs:395,419-420`) and renders the captured source. Progress sends are `let _ =`, so a dead channel is ignored. Plan 0099: "Cancel Quit keeps the export running". After a reload the export finishes and publishes. A new export answers `busy` (`:31-35`), and a Quit is refused with "an audio export is still running" (`app_quit.rs:76-82`). |
| Exit does not reset `QuitState` | **Intended** | AGENTS.md: "An approved native exit holds the export permit through Tauri shutdown". Plan 0099 `proposal.md` refinement: "The final exit takes the same native permit as export start and holds it until process shutdown". Releasing it on exit would let an export start between approval and exit. |
| Reload's `QuitState::detach` keeps `exit_permit` | Intended | A permit means exit is already approved. `request` ignores new requests while it is held (`app_quit.rs:44`). |
| Exit does not call `Bridge::detach` | Harmless | The process ends, and `close_server` stops the accept task. Pending oneshots drop with the process. |
| Reload does not close the socket | Intended | The comment at `lib.rs:37` says requests answer `not ready` until the page attaches again. |
| Reload stops playback **last**; exit stops it **first** | Benign, undocumented | On reload, a recorder finalize (WAV and sidecar write) can delay the stop by that long. A take's song backing plays on superdough in the webview (AGENTS.md), so this rarely overlaps with Rust playback. |
| Watcher and piano roll not in the hooks | Intended / harmless | The project stays open across a reload (the watcher keeps emitting to the new page). The piano roll inspection is bounded. On exit, the process ending stops both. |
| Reload during an export, then window close | **Gap that predates this task (not caused by the hooks)** | After a reload the export dialog is gone, so `canCloseWindow` returns true (`shell.ts:248`). Rust does not check `ExportState` on close, so closing the window kills the running export worker. It is an edge case: reload is HMR or Cmd-R, and no control method reloads the page. It is out of scope here. |

## 3. Tests today and feasibility

- `playback.rs:376-382` `end_session_stops_the_session_and_is_idempotent`.
- `control.rs:1267` `detach_fails_pending_requests_and_later_ones_are_not_ready`.
- `app_quit.rs` tests (around `:261-300`): duplicate, cancelled or stale requests including
  `detach` (`:277`); an active export blocks exit; a delivery error.
- `crates/strudeland-record/src/takes/tests.rs:712-713,864-912` (reload and quit per state), and
  the transcript `src-tauri/src/recording/transcript.rs:323-330`.
- **No test covers the hooks themselves.** Nothing checks which states each hook touches or in
  what order.

Every state can be built without an `AppHandle`: `QuitState::default`, `Bridge::new`,
`PlaybackState::default`, and `Takes` through `strudeland_record::takes::testing::TakesRig`
(already used by `recording/rig.rs`). `control::shutdown(&AppHandle)` only looks up the bridge,
so a lifecycle module calls `Bridge::close_server` (private, same crate) directly.

That gives two test options:
- **Coverage with the real states.** After `on_reload` or `on_exit`, check that quit is not
  ready, the bridge is detached and pending requests failed, takes are idle, and a playback stop
  is `stale`. Closing the socket is a no-op when nothing is served.
- **Order.** Testing the order needs a seam: a small trait or a step log over the four
  participants, with fakes in the test. Only the exit order is a stated invariant ("audio stops
  first", "recorder before socket").

## 4. Visible behaviour

The refactor keeps all of these unchanged:
- reload order Q, B, R, P;
- exit order P, R, C;
- the `main` / `Started` filter and the `try_state` guards (or their equivalent);
- stderr (`control:` lines from `start`, the recorder's forwarder panic message);
- no events emitted.

Decisions that would change behaviour:

| Decision | Recommended |
|---|---|
| Cancel or abandon a running export on reload | **Keep: no.** There is no cancel mechanism, and plan 0099 keeps exports running. |
| Release the exit permit or reset `QuitState` on exit | **Keep: no.** This is the documented invariant. |
| Align reload order with exit (playback first) | **Keep the current order.** If aligned later, do it as a separate, stated change. |
| Stop the watcher or drop the piano roll host on reload or exit | **Keep: no.** |
| Fix the reload-then-close gap that kills an export | **Out of scope.** File it separately if wanted: a native close check or a re-attach of the export state. |

## 5. Size and worth

The module is about 40 lines (`src-tauri/src/lib.rs` down to two calls, plus
`src-tauri/src/lifecycle.rs`), with about 60-100 lines of tests. That is small, and the glue is
the only untested part of `lib.rs`. The value is modest: four calls, already commented, and
stable since 63b945d. The main gains are:
- one documented place that states the export and `QuitState` decisions;
- a coverage test that fails when a new stateful subsystem is managed but missing from a hook.

## 6. New callers or copies since 63b945d

There are none. `end_session`, `on_reload`, `Takes::shutdown`, `Bridge::detach`,
`control::shutdown` and `QuitState::detach` are called only from `lib.rs` and the tests listed
above. No second lifecycle hook was added. The commits touching `lib.rs` since 63b945d are #34,
#40 and #43 plus the instruments and piano roll features, and none of them changed hook
behaviour.

## Recommendation

**Proceed, with a narrow scope:**
- Add `src-tauri/src/lifecycle.rs` with `on_reload(&QuitState, &Bridge, Option<&Takes>,
  Option<&PlaybackState>)` and `on_exit(Option<&PlaybackState>, Option<&Takes>, &Bridge)`.
  `lib.rs` keeps only the label and event filter and the state lookups.
- Put the order rationale and the intended asymmetries (export, `QuitState` permit, socket kept
  across reload, watcher and piano roll left alone) in the module docs.
- Test coverage with the real states.
- Test the exit order only through a minimal seam (a trait or step log). If that seam is more
  than about 20 lines, drop the order test and rely on the docs.
- Keep behaviour identical: answer every decision in section 4 with "keep".
- Optional: `control::shutdown` becomes unused and can be removed.

Cancelling is a reasonable alternative if the order test needs a heavier seam than that.

## Decisions (2026-11-27, recommended answers taken; behaviour unchanged)

1. A reload does not cancel an export. Exports have no cancel mechanism and run to completion.
2. On exit, `QuitState` is not reset and the export permit is not released: the approved exit holds the permit until the process ends.
3. The reload order stays as it is, with playback stopped last.
4. The watcher, the piano roll host and the instrument preview are left alone.
5. Filed separately: a reload during an export, followed by a window close, kills the export.

**Scope:**
- A new `src-tauri/src/lifecycle.rs` provides `on_reload` and `on_exit` over the managed states, and its module docs record the order and decisions 1-4.
- Coverage tests run over the real states.
- An exit-order test is added only if its seam stays under ~20 lines.
- `control::shutdown` is removed if it becomes unused.
