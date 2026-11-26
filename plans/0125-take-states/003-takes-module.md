# 003: the takes module in strudeland-record

## Goal

Move the take lifecycle (stage 002's code) out of the app crate into
`crates/strudeland-record/src/takes/`. The moved code is:

- `TakeState` and `Inner`;
- the verbs;
- the forwarder;
- sidecar building;
- the prefs file;
- the repair skip.

Two types are added:

- **`Takes`**, the struct the verbs become methods of, over the three existing ports;
- **`TakeError`**, the module's error, mapped once to `CommandError` in `recording.rs`.

The move also injects the sidecar's `app.version`. The lifecycle tests move with the code, as
transition tables. The app keeps the wire types, the adapters, the commands and about 16 tests.
Nothing visible changes, and the transcript's expected JSON is untouched.

## Prerequisites

- Stage 002 is committed.
- Read the proposal's "Module layout", "The ports and the config", "`TakeError`" and "The app
  after stage 003".

## Files

- `crates/strudeland-record/Cargo.toml`:
  - move `strudeland-project = { path = "../strudeland-project" }` from `[dev-dependencies]` to
    `[dependencies]`;
  - add `serde = { version = "1.0.229", features = ["derive"] }` and
    `serde_json = { version = "1.0.154", features = ["float_roundtrip"] }`.
- `Cargo.lock`: only the record package's dependency list changes (`cargo test --locked` fails
  otherwise). Run `cargo build -p strudeland-record` once without `--locked` to update it.
- `crates/strudeland-record/src/lib.rs`:
  - `pub mod takes;`;
  - the crate doc gains one sentence on `takes`;
  - `testing` re-exports `takes::testing::{FakePlatform, CollectingSink, fake_maker}`.
- `crates/strudeland-record/src/takes/{mod,error,sidecar,prefs,tests}.rs` (new).
- `src-tauri/src/recording.rs`: the adapter (below).
- `src-tauri/src/recording/rig.rs`: rebuilt on `strudeland_record::testing`.
- `src-tauri/src/recording/transcript.rs`: the driver calls the new API. **The expected block
  does not change.**
- `src-tauri/src/command_error.rs`: `for_memo_error(&ProjectError)`, with `for_memos` calling
  it.
- `src-tauri/src/lib.rs`: `recording::on_reload(&recorder.0)` becomes `recorder.0.on_reload()`,
  and `recording::shutdown(&recorder.0)` becomes `recorder.0.shutdown()`.
- `AGENTS.md`: the voice memo layout bullet.
- `crates/strudeland-record/README.md`: a "Takes" section, the intro line and the licences line.

## Implementation

1. **Move, don't rewrite.** Move stage 002's code into the files the proposal's layout gives,
   keeping every statement's order. The free functions over `&Core` become methods on `Takes`.
   `Core` is renamed `Takes`, with the same fields plus `config: TakesConfig`.
   - `arm` takes `self: &Arc<Self>`, for `ensure_recorder`'s `Weak`.
   - `forward(weak, events)` calls `takes.on_event(event)` for each event.
   - `on_event` dispatches:
     - `Level` goes to `sink.level(level)` without locking;
     - a non-user `TakeEnded` goes to `ended_by_itself`;
     - `Failed` goes to `failed`;
     - anything else does nothing.
   - `RecorderState(pub Arc<Takes>)` stays in the app.
2. **Errors.**
   - `takes/error.rs` holds `TakeError`, exactly the proposal's table. The two restricted texts
     are `cfg`'d by `target_os = "macos"` as today's `RESTRICTED`.
   - Every `CommandError::busy(BUSY)`, `other(NOT_ARMED)` and similar call in the moved code
     becomes the matching `TakeError` variant. `.map_err(CommandError::for_memos)` becomes
     `.map_err(|e| TakeError::Memo(Arc::new(e)))`. A `RecordError` `?` uses
     `From<RecordError> for TakeError` (`Record(Arc::new(e))`).
   - `ensure_permission`'s refusal is `PermissionRefused(permission)`. A `NotDetermined` after
     the request maps to the denied text, as today.
   - Every `CommandError` in an emitted event becomes a `TakeError` in a `TakeEvent`. The cap
     gives `LimitReached`, a write error `WriteFailed(message)`, and `Failed`
     `Record(Arc::new(error))`.
   - `start` returns `Result<String, E>` where `E: From<TakeError>`. The resolver's error is
     returned as it is.
3. **Domain types** (`takes/mod.rs`):
   - `TakeStart`, `TakeStop`, `TakeStatus`, `Devices`, `Selection` and `TakeEvent`, as in the
     proposal;
   - `RecState` and `Permission` with `Serialize` and `rename_all = "camelCase"`. Drop their
     `cfg_attr(... allow(dead_code))` attributes, because public items of a library are never
     dead;
   - `RepairOutcome`, moved unchanged.

   `ArmedInput.device` is a `DeviceInfo`, built as `DeviceInfo { channels: info.channels,
   sample_rate: info.sample_rate, ..device.clone() }`. `TakeMeta.app_version` is copied from
   `self.config.app_version` at start, and `build_sidecar` writes it.
4. **`testing`** (`takes/mod.rs`, `#[doc(hidden)] pub mod testing`), re-exported from the crate's
   `testing`:
   - `FakePlatform`: the app's fake, moved with public fields. It has `new()` (`has_prompt`
     true, `authorized`, devices `mic()`, `usb()`) and helpers `mic()`, `usb()` and
     `device(...)`;
   - `CollectingSink`: `states() -> Vec<TakeEvent>`, `last()`, `wait(pred)` and `levels()`;
   - `fake_maker(limits, arm_failure: Arc<Mutex<Option<RecordError>>>) -> (RecorderMaker,
     Receiver<FakeHandle>)`, the app rig's maker closure, moved.
5. **`recording.rs`**: what the proposal's "The app after stage 003" keeps.
   - `RecordingStatus`, `DevicesResult`, `InputSelection` and `StateEvent` each get
     `From<domain>`. `StartFields` and `StopFields` get `From<wire> for domain`, and
     `StopTiming`'s fields flatten into `TakeStop`.
   - `fn takes_config() -> TakesConfig { TakesConfig { app_version:
     env!("CARGO_PKG_VERSION").to_string() } }`. `system_state` passes it.
   - `record_error(&RecordError)` holds today's table. The four overridden messages come from
     `TakeError::{Busy, NotArmed, NoAudio, PermissionRefused(Permission::Denied)}.to_string()`.
     `From<RecordError>` calls it. `From<TakeError>` holds the proposal's table, with `Record`
     going to `record_error(&e)` and `Memo` to `CommandError::for_memo_error(&e)`.
   - The text constants (`BUSY` … `LIMIT`) are deleted from `recording.rs`; `TakeError` owns
     them.
   - Each command calls the method and maps the result. `recording_start` is
     `takes.start(|| projects.current_for(&root), song_stem.as_deref(), fields.into())
     .map(|stem| StartResult { stem })`.
   - `repair_in_background` spawns `takes::repair_on_open(&takes, &project)`.
   - Update the module header: the logic lives in `strudeland_record::takes`, and this file is
     the wire, the permission, the sink and the commands.
6. **`rig.rs`** builds `Takes::new(Box::new(Arc::clone(&platform)), sink, maker,
   takes_config())` from the `testing` fakes. Its sink is a `CollectingSink`, and it exposes
   `states() -> Vec<StateEvent>` mapped through `StateEvent::from`, so the transcript records
   wire events through the production mapping. Delete the app's own `FakePlatform` and
   `FakeSink`.
7. **`transcript.rs`**: change only the driver calls:
   - `arm(&rig.core, …)` becomes `rig.takes.arm(…).map(RecordingStatus::from)`;
   - an injected `Failed` becomes `rig.takes.on_event(Event::Failed { … })`, and an injected
     `TakeEnded` becomes `on_event(Event::TakeEnded(…))`;
   - `ProjectState::default()` for `start-no-project` becomes the resolver
     `|| ProjectState::default().current_for(&root)`.

   The normalizer stays, `<app version>` included, which now also pins the injected version.

## Tests

Replace; don't layer. Every moved test keeps its assertions. Assertions on `CommandError` values
become assertions on `TakeError` variants. Their texts move to the error table, and the
transcript pins each event's mapped text. Where a test asserted `env!("CARGO_PKG_VERSION")`,
the moved test asserts `"test"`.

**Into `takes/` as unit tests** (inline `#[cfg(test)] mod tests` in the file of the code):

- `prefs.rs`: `prefs_parse_errors_give_defaults`, `prefs_round_trip`, `selection_resolution` and
  `saved_channel_cases`.
- `sidecar.rs`: `bar1_frame_cases` and `bar1_frame_above_two_to_the_53`.
- `mod.rs`: `raw_permission_values`.
- `error.rs`: `take_error_texts`, a new test that lists every variant's `Display`. It is the
  only text table in the record crate.

**Into `takes/tests.rs`**, the lifecycle, as transition tables over `FakeSource`,
`FakePlatform`, `CollectingSink`, a temp `Project` and `TakesConfig { app_version: "test" }`.
Each table is a `Vec` of rows `{ name, setup, act, result, events, state }`, run by one loop.
A test whose body is a sequence becomes a row whose `act` is that sequence. Group them as
follows:

| table / test | old tests (from `recording.rs`) |
|---|---|
| `permission_rows` | `not_determined_asks_then_arms_when_granted`, `not_determined_asks_then_refuses_when_denied`, `denied_and_restricted_open_no_stream`, `on_a_desktop_without_a_prompt_the_stream_decides_access_and_can_be_retried` |
| `a_second_arm_during_a_pending_request_is_busy` (stays a test: threads) | same |
| `arm_rows` | `arm_without_devices_is_no_device`, `arm_with_an_absent_device_id_is_no_device`, `a_busy_device_stays_idle_and_reports_the_device_error` (asserts `TakeError::Record(DeviceBusy)`; the kind split is the app's error table), `a_channel_out_of_range_is_other`, `arm_reports_the_armed_input_and_rearm_replaces_it`, `arm_and_disarm_during_a_take_are_busy`, `disarm_closes_the_stream_and_emits_idle` |
| `prefs_rows` | `a_saved_exclusive_input_keeps_its_identity_and_armed_channel_count`, `arm_writes_prefs_only_when_it_carried_an_argument`, `a_prefs_write_failure_does_not_fail_the_arm`, `devices_and_idle_status_use_the_saved_choice` |
| `levels_are_forwarded_while_armed` (stays a test: polling) | same |
| `start_rows` | `start_names_the_take_and_reports_recording`, `start_while_recording_is_busy`, `start_unarmed_is_other`, `start_with_a_stale_root_touches_nothing` (the resolver returns a test error type; assert it comes back unchanged and nothing is touched), `a_start_without_audio_times_out_and_leaves_no_partial`, `an_invalid_start_tempo_still_saves_the_memo` |
| `stop_rows` | `stop_writes_the_webview_fields_input_and_anchors` (app version `"test"`), `stop_without_fields_writes_the_fallback_sidecar`, `invalid_webview_fields_keep_the_audio_with_the_fallback`, `a_bar1_outside_the_take_is_stored_null`, `a_second_stop_is_other`, `stop_finalizes_into_the_project_captured_at_start` (resolver over project A; B stays empty) |
| `publish_rows` | `a_failed_publish_keeps_the_take_and_a_retry_publishes_it`, `a_take_that_failed_to_publish_is_published_by_disarm_reload_and_quit` (rows for disarm, reload, quit, plus arm and start, matching the transcript's settle flows) |
| `ending_rows` | `the_length_cap_finalizes_and_a_later_stop_is_idempotent`, `a_device_loss_finalizes_and_emits_idle`, `a_device_loss_while_armed_emits_idle_without_a_stem`, `a_stale_failed_after_a_rearm_is_ignored` (through `on_event`), `a_rearm_after_a_device_loss_stays_armed`, `arm_forgets_an_ended_take` |
| `lifecycle_rows` | `a_reload_finalizes_the_take_and_disarms_silently`, `a_reload_while_idle_is_a_no_op`, `quit_finalizes_the_take_and_joins` |
| `repair_rows` | `repair_skips_a_project_with_a_take_in_progress_or_ended`, `repair_runs_in_another_project_during_a_take`, `a_repair_error_is_reported_not_raised` |
| `leave_rows` | stage 002's three `leave` tests |

`takes/tests.rs` has private access, as stage 002's tests had, so `armed_id`, `recorder.is_none()`
and `forwarder.is_none()` keep their direct reads.

**Stays in the app** (`recording.rs` `mod tests`, plus the transcript), with the counts:

1. `project_for_is_typed_for_a_stale_root_and_no_project`.
2. `project_errors_for_memos`.
3. `errors_map_to_command_errors`. It replaces `errors_from_record_errors` and covers every
   `RecordError` row (`cfg`'d `DeviceBusy`, as today) and every `TakeError` variant, including
   `Memo` and `PermissionRefused` with each permission. Kinds and messages are written as
   literal strings, not constants.
4. `a_busy_take_serializes_as_kind_and_message`.
5. `status_and_event_wire_shapes`, extended with `StateEvent::from(TakeEvent)` for an event
   without and with an error, and `RecordingStatus::from(TakeStatus)`. Its existing literals
   are unchanged.
6. `stop_fields_parse_host_times_above_two_to_the_53`.
7. `the_avfoundation_status_reads_without_prompting` (macOS).
8. `the_bundle_declares_the_microphone`.
9. `the_opener_scope_adds_only_the_microphone_settings_url`.
10. `opening_a_project_repairs_it_in_the_background`.
11. `list_and_stop_return_the_project_entry_camel_case`.
12. `library_commands_reject_a_stale_root`.
13. `library_commands_work_and_map_errors`.
14. `host_now_is_a_decimal_string`.
15. `a_stop_writes_the_app_version`: a new test through the app rig, so through `takes_config()`.
    `sidecar.app.version == env!("CARGO_PKG_VERSION")` of `strudeland-app`, and it is not
    `"0.1.0"`, the record crate's version.
16. `transcript_of_every_take_flow`.

Count check: `cargo test -p strudeland-app --lib recording::` runs 16 tests. The record crate
gains the moved tests: 49 old plus 4 from stage 002 plus `take_error_texts`. Log both counts in
the crates log.

## Docs

- **`AGENTS.md`, the voice memo layout bullet.** After "the threads, format and the
  capture-timestamp decision)" add:

  > `crates/strudeland-record/src/takes/` the take lifecycle (`Takes` over the `Platform`,
  > `Sink` and `RecorderMaker` ports; `TakeState`, idle, armed, recording or ended; the
  > sidecar build, the device prefs file and the repair skip; `TakeError`; the fakes in
  > `testing`).

  Change "`src-tauri/src/recording.rs` the recording commands, events, `RecorderState` and the
  microphone permission" to "`src-tauri/src/recording.rs` the wire types, the mapping of
  `TakeError` and `TakeEvent` to `CommandError` and `recording://state`, `SystemPlatform` (the
  microphone permission), `AppSink`, `RecorderState` and the commands". The rest of the bullet
  stays.
- **`crates/strudeland-record/README.md`:**
  - Intro: "No Tauri dependency. The take lifecycle is `takes`; the app wiring (wire types,
    permission, commands) is `src-tauri/src/recording.rs`; the sidecar and file naming live in
    `strudeland-project`."
  - New "## Takes" section after "Threads":
    - the four states and what leaves each one;
    - `leave` publishing an ended take that is not yet published;
    - `pending_loss_stem` as the bridge between `TakeEnded(DeviceLost)` and its `Failed`;
    - the lock rule (held across arm, start, stop; levels and the permission request outside
      it);
    - the forwarder's `Weak`, and the join outside the lock;
    - the version the app passes in `TakesConfig`;
    - "the quirks the app keeps are tracked in the project's task list", with no task number in
      the README.
  - Licences: add serde and serde_json (MIT or Apache-2.0).

## Validation

- The common gates in [000-prompt.md](000-prompt.md), all `exit 0`, **including the
  transcript-diff gate**.
- `git diff d38b30e -- src/lib/ipc/bindings.ts` is empty, and `bun test` passes, the bindings
  staleness test included.
- `grep -n "CARGO_PKG_VERSION" crates/strudeland-record/src` finds nothing.
  `grep -n "CARGO_PKG_VERSION" src-tauri/src/recording.rs` finds `takes_config` and the test.
- `grep -rn "tauri" crates/strudeland-record/src` finds nothing.
- `cargo tree -p strudeland-record -e normal | grep -c tauri` is 0.
- **Code behind `cfg`:**
  - `takes/error.rs`'s two restricted texts: each arm is used on its OS, and the error test
    lists the current OS's text.
  - The `DeviceBusy` split stays in `record_error`. On Windows and Linux both arms compile. The
    error table's `cfg`'d rows are unchanged.
  - `recording.rs`'s `permission` module: `use strudeland_record::takes::Permission`. On Linux
    it is used by `status()` (`NotDetermined`). On Windows it is used by the WinRT match. On
    macOS `Permission::from_raw` is called, and it is a public method of the record crate, so no
    `dead_code` warning anywhere.
  - Nothing else in the app that refers to `Permission` is behind `cfg`. Check that no import
    becomes unused on non-macOS: an import that only the macOS branch uses is a Windows and
    Linux clippy error.
  - Try the Windows-target clippy (000-prompt) and log it.

## STOP conditions

- The transcript-diff log shows any line.
- A wire literal in `status_and_event_wire_shapes` would change, or a message in the error
  table differs from today's text.
- `app.version` in any app-side sidecar is not `strudeland-app`'s version.
- A moved test needs a new wait, retry or sleep.
- `Cargo.lock` changes beyond `strudeland-record`'s dependency list.
- The record crate would need a Tauri, `objc2` or `windows` dependency.

## Commit message

```
refactor(record): move the take lifecycle into strudeland-record

Takes owns the recorder's state machine over the platform, sink and recorder
ports, with its own error type that the app maps to CommandError in one
table. The app keeps the wire types, the permission APIs, the event sink and
the commands, and passes its version in for the sidecar. The lifecycle tests
move with the code and no longer build the Tauri crate. Events, results,
sidecars and files are unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
