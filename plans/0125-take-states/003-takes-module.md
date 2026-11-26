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

The move also injects the sidecar's `app.version`. The lifecycle tests move with the code, each
as its own `#[test]` function under its old name. The app keeps the wire types, the adapters,
the commands and 16 tests (15 off macOS).
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
    `serde_json = { version = "1.0.154", features = ["float_roundtrip"] }`. `float_roundtrip` is
    harmless here: `strudeland-project` already turns it on for the one `serde_json` in the
    graph, so feature unification gives every crate, the app included, the same parser as
    today.
- `Cargo.lock`: only the record package's dependency list changes (`cargo test --locked` fails
  otherwise). Run `cargo build -p strudeland-record` once without `--locked` to update it.
- `crates/strudeland-record/src/lib.rs`:
  - `pub mod takes;`;
  - the crate doc gains one sentence on `takes`;
  - `testing` re-exports `takes::testing::{FakePlatform, CollectingSink, fake_maker}`, the
    one public path to them.
- `crates/strudeland-record/src/takes/{mod,error,sidecar,prefs,testing,tests}.rs` (new).
  `testing.rs` is `pub(crate) mod testing` in `takes/mod.rs`, so the fakes are defined once and
  reached only through `strudeland_record::testing`, as `FakeSource` is.
- `src-tauri/src/recording.rs`: the adapter (below).
- `src-tauri/src/recording/rig.rs`: rebuilt on `strudeland_record::testing`.
- `src-tauri/src/recording/transcript.rs`: the call adapters call the new API. **The frozen
  region and the expected block do not change.**
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
     `From<RecordError> for TakeError`, which is the proposal's mapping: `AlreadyRecording` to
     `Busy`, `NotArmed` to `NotArmed`, `StartTimeout` to `NoAudio`, `PermissionDenied` to
     `PermissionRefused(Permission::Denied)`, and every other variant to `Record(Arc::new(e))`.
   - Two places read the `RecordError` before it converts, as today: `arm` matches
     `RecordError::PermissionDenied` from `arm_with_id` to set `Idle { stream_denied: true }`,
     and the log lines that print a `RecordError` with `{e}` (`:973`, `:1055`) print it before
     the `?` or `map_err`, so their text is unchanged.
   - `ensure_permission`'s refusal is `PermissionRefused(permission)`. A `NotDetermined` after
     the request maps to the denied text, as today.
   - Every `CommandError` in an emitted event becomes a `TakeError` in a `TakeEvent`. The cap
     gives `LimitReached`, a write error `WriteFailed(message)`, and `Failed`
     `TakeError::from(error)` (a `DeviceLost` or `Backend`, so `Record`).
   - `TakeError` cannot derive `PartialEq` (`RecordError` and `ProjectError` hold an
     `io::Error`). It gets a manual `impl PartialEq for TakeError`: equal when the variants are
     the same (`std::mem::discriminant`) and the `Display` texts are equal. `TakeEvent` then
     derives `PartialEq`, and every moved assertion compares `TakeError` and `TakeEvent` values
     with `assert_eq!`.
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
4. **`testing`** (`takes/testing.rs`, declared `pub(crate) mod testing;` in `takes/mod.rs`). The
   fakes are defined there once and re-exported from the crate's `#[doc(hidden)] pub mod
   testing`, which is the only public path to them (as for `FakeSource`):
   - `FakePlatform`: the app's fake, moved with public fields. It has `new()` (`has_prompt`
     true, `authorized`, devices `mic()`, `usb()`) and helpers `mic()`, `usb()` and
     `device(...)`;
   - `CollectingSink`: `states() -> Vec<TakeEvent>`, `last()`, `wait(pred)` and `levels()`.
     `wait` keeps the app fake's 5 s deadline and 2 ms poll, unchanged;
   - `fake_maker(limits, arm_failure: Arc<Mutex<Option<RecordError>>>) -> (RecorderMaker,
     Receiver<FakeHandle>)`, the app rig's maker closure, moved.
5. **`recording.rs`**: what the proposal's "The app after stage 003" keeps.
   - `RecordingStatus`, `DevicesResult`, `InputSelection` and `StateEvent` each get
     `From<domain>`. `StartFields` and `StopFields` get `From<wire> for domain`, and
     `StopTiming`'s fields flatten into `TakeStop`.
   - `From<TakeEvent> for StateEvent` builds through `StateEvent::new`, `.stem` and `.error`
     (`recording.rs:227-245`), so those builders stay used in the non-test build and no
     `dead_code` warning appears. If the implementation finds a direct struct literal clearer,
     it deletes the three builders instead (or makes them `#[cfg(test)]` if a test still uses
     them); either way the non-test build has no dead code.
   - `impl From<Level> for LevelEvent` (`peak`, `rms`). `AppSink::level` emits
     `LevelEvent::from(level)`.
   - `fn takes_config() -> TakesConfig { TakesConfig { app_version:
     env!("CARGO_PKG_VERSION").to_string() } }`. `system_state` passes it.
   - `record_error(&RecordError) -> CommandError` holds today's table without the four
     overrides: `NoDevice`, the `cfg`'d `DeviceBusy` split, `DeviceLost`, and `other(e.to_string())`
     for the rest. `impl From<RecordError> for CommandError` is
     `CommandError::from(TakeError::from(e))`, so `AlreadyRecording`, `NotArmed`,
     `StartTimeout` and `PermissionDenied` reach the wire through their `TakeError` variants,
     with kinds and texts identical to today's (`recording.rs:62`, `:70-72`). `From<TakeError>`
     holds the proposal's table, with `Record` going to `record_error(&e)` and `Memo` to
     `CommandError::for_memo_error(&e)`. Every `TakeError`'s `Display` equals the message of
     the `CommandError` it maps to.
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
7. **`transcript.rs`**: change only the call adapters, above `// --- frozen: begin ---`. The
   frozen region (`normalize`, the restricted literals, the flows) and the expected block are
   untouched, and 000-prompt's gate diffs both:
   - `arm(&rig.core, …)` becomes
     `rig.takes.arm(…).map(RecordingStatus::from).map_err(CommandError::from)`, and every other
     verb's adapter maps the same way;
   - `inject_failed` becomes `rig.takes.on_event(Event::Failed { … })`, and `inject_ended`
     becomes `on_event(Event::TakeEnded(…))`. Both still run on the test thread, synchronously;
   - `ProjectState::default()` for `start-no-project` becomes the resolver
     `|| ProjectState::default().current_for(&root)`.

   The normalizer stays, `<app version>` included, which now also pins the injected version.
   The restricted text is a literal in the frozen region, so deleting `RESTRICTED` from
   `recording.rs` does not touch it.

## Tests

Replace; don't layer. Every lifecycle test moves as its own `#[test]` function under the same
name, with the same steps and the same assertions. Only two things change in a moved body: the
call syntax (`arm(&rig.core, …)` becomes `rig.takes.arm(…)`) and the error assertions.
Assertions on `CommandError` values become assertions on `TakeError` values (through the manual
`PartialEq`), and events are `TakeEvent`s. Their texts move to the error table, and the
transcript pins each event's mapped text. Where a test asserted `env!("CARGO_PKG_VERSION")`,
the moved test asserts `"test"`.

**Into `takes/` as unit tests** (inline `#[cfg(test)] mod tests` in the file of the code):

- `prefs.rs`: `prefs_parse_errors_give_defaults`, `prefs_round_trip`, `selection_resolution` and
  `saved_channel_cases`.
- `sidecar.rs`: `bar1_frame_cases` and `bar1_frame_above_two_to_the_53`.
- `mod.rs`: `raw_permission_values`.
- `error.rs`: `take_error_texts`, a new test that lists every variant's `Display` (`NoAudio`
  and the `cfg`'d restricted text included) and the four `From<RecordError>` mappings. It is the
  only text table in the record crate.

**Into `takes/tests.rs`**, the lifecycle, over `FakeSource`, `FakePlatform`, `CollectingSink`,
a temp `Project` and `TakesConfig { app_version: "test" }`. No test is merged, split or renamed.
The groups below are only the order in the file (a `// ---` comment per group):

| group | moved tests (from `recording.rs`), one `#[test]` each |
|---|---|
| permission | `not_determined_asks_then_arms_when_granted`, `not_determined_asks_then_refuses_when_denied`, `denied_and_restricted_open_no_stream`, `on_a_desktop_without_a_prompt_the_stream_decides_access_and_can_be_retried` |
| permission (threads) | `a_second_arm_during_a_pending_request_is_busy` |
| arm and disarm | `arm_without_devices_is_no_device`, `arm_with_an_absent_device_id_is_no_device`, `a_busy_device_stays_idle_and_reports_the_device_error` (asserts `TakeError::Record(DeviceBusy)`; the kind split is the app's error table), `a_channel_out_of_range_is_other`, `arm_reports_the_armed_input_and_rearm_replaces_it`, `arm_and_disarm_during_a_take_are_busy`, `disarm_closes_the_stream_and_emits_idle` |
| prefs | `a_saved_exclusive_input_keeps_its_identity_and_armed_channel_count`, `arm_writes_prefs_only_when_it_carried_an_argument`, `a_prefs_write_failure_does_not_fail_the_arm`, `devices_and_idle_status_use_the_saved_choice` |
| levels (polling) | `levels_are_forwarded_while_armed` |
| start | `start_names_the_take_and_reports_recording`, `start_while_recording_is_busy`, `start_unarmed_is_other`, `start_with_a_stale_root_touches_nothing` (the resolver returns a test error type; assert it comes back unchanged and nothing is touched), `a_start_without_audio_times_out_and_leaves_no_partial`, `an_invalid_start_tempo_still_saves_the_memo` |
| stop | `stop_writes_the_webview_fields_input_and_anchors` (app version `"test"`), `stop_without_fields_writes_the_fallback_sidecar`, `invalid_webview_fields_keep_the_audio_with_the_fallback`, `a_bar1_outside_the_take_is_stored_null`, `a_second_stop_is_other`, `stop_finalizes_into_the_project_captured_at_start` (resolver over project A; B stays empty) |
| publish | `a_failed_publish_keeps_the_take_and_a_retry_publishes_it`, `a_take_that_failed_to_publish_is_published_by_disarm_reload_and_quit` (unchanged in scope: the transcript's settle flows cover arm and start) |
| endings | `the_length_cap_finalizes_and_a_later_stop_is_idempotent`, `a_device_loss_finalizes_and_emits_idle`, `a_device_loss_while_armed_emits_idle_without_a_stem`, `a_stale_failed_after_a_rearm_is_ignored` (through `on_event`), `a_rearm_after_a_device_loss_stays_armed`, `arm_forgets_an_ended_take` |
| reload and quit | `a_reload_finalizes_the_take_and_disarms_silently`, `a_reload_while_idle_is_a_no_op`, `quit_finalizes_the_take_and_joins` |
| repair | `repair_skips_a_project_with_a_take_in_progress_or_ended`, `repair_runs_in_another_project_during_a_take`, `a_repair_error_is_reported_not_raised` |
| leave | stage 002's `leaving_an_unpublished_ended_take_publishes_it`, `leaving_a_published_ended_take_writes_nothing`, `leaving_a_state_that_is_not_ended_publishes_nothing` |

`takes/tests.rs` has private access, as stage 002's tests had, so `armed_id`, `recorder.is_none()`
and `forwarder.is_none()` keep their direct reads.

Quirk 1 (a `recorder.stop()` error drops the take) has no test, today or after the move:
`Recorder::stop` fails only when no take is running and no stored result is left
(`recorder.rs:437-443`), and the lock plus the forwarder's `take_if` keep a `Recording` state
from reaching it with `FakeSource`. It is kept by code review: the moved `stop` keeps the
`Armed(armed)` and the early return of stage 002, line for line.

**Stays in the app** (`recording.rs` `mod tests`, plus the transcript), with the counts:

1. `project_for_is_typed_for_a_stale_root_and_no_project`.
2. `project_errors_for_memos`.
3. `errors_map_to_command_errors`. It replaces `errors_from_record_errors` and covers every
   `RecordError` row (`cfg`'d `DeviceBusy`, as today) and every `TakeError` variant, including
   `Memo` and `PermissionRefused` with each permission. Kinds and messages are written as
   literal strings, not constants.
4. `a_busy_take_serializes_as_kind_and_message`. `CommandError::busy(BUSY)` becomes
   `CommandError::from(TakeError::Busy)`; the expected JSON literal is unchanged.
5. `status_and_event_wire_shapes`, extended with `StateEvent::from(TakeEvent)` for an event
   without and with an error, `RecordingStatus::from(TakeStatus)`, and
   `LevelEvent::from(Level { peak: 0.5, rms: 0.25 })` against the existing `{"peak":0.5,"rms":0.25}`
   literal. The `LIMIT` argument becomes `TakeError::LimitReached` through `From`. Its existing
   expected literals are unchanged.
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

Count check: `cargo test -p strudeland-app --lib recording::` runs 16 tests on macOS and 15
elsewhere (test 7 is `cfg(target_os = "macos")`); the crates log holds the macOS count.
`cargo test -p strudeland-record --lib takes::` runs exactly 54 `#[test]` functions: the 49
moved (63 minus the 14 old ones that stay), the 4 from stage 002 and `take_error_texts`.
None of them is `cfg`'d, so 54 on every OS. Log both counts in
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
  - Licences: add serde and serde_json (MIT or Apache-2.0), and libc (MIT or Apache-2.0, the
    Linux-only dependency the line omits today).

## Validation

- The common gates in [000-prompt.md](000-prompt.md), all `exit 0`, **including the
  transcript-diff and frozen gates**.
- `git diff d38b30e -- src/lib/ipc/bindings.ts` is empty, and `bun test` passes, the bindings
  staleness test included.
- `grep -rn "CARGO_PKG_VERSION" crates/strudeland-record/src` finds nothing.
  `grep -rn "CARGO_PKG_VERSION" src-tauri/src/recording.rs src-tauri/src/recording/` finds
  `takes_config` and the test.
- `grep -rnE "tauri::|use tauri" crates/strudeland-record/src` finds nothing. Prose that names
  the app (`src-tauri/…` in a doc comment) is allowed; code is not.
- `grep -nE "^const (BUSY|PERMISSION_PENDING|NOT_ARMED|NO_TAKE|NO_AUDIO|DENIED|RESTRICTED|LIMIT):" src-tauri/src/recording.rs`
  finds nothing: the texts live in `TakeError`.
- `cargo tree -p strudeland-record -e normal | grep -c tauri` is 0.
- **Code behind `cfg`:**
  - `takes/error.rs`'s two restricted texts: each arm is used on its OS, and the error test
    lists the current OS's text.
  - The `DeviceBusy` split stays in `record_error`. Each OS compiles one arm: macOS's gate sees
    the `DeviceLost` one, and the PR's Windows and Linux CI the `busy` one. The error table's
    `cfg`'d rows are unchanged.
  - `recording.rs`'s `permission` module: `use strudeland_record::takes::Permission`. On Linux
    it is used by `status()` (`NotDetermined`). On Windows it is used by the WinRT match. On
    macOS `Permission::from_raw` is called, and it is a public method of the record crate, so no
    `dead_code` warning anywhere.
  - Nothing else in the app that refers to `Permission` is behind `cfg`. Check that no import
    becomes unused on non-macOS: an import that only the macOS branch uses is a Windows and
    Linux clippy error.
  - Try the Windows-target clippy (000-prompt) and log it.
  - Push the commit to the draft PR: `native.yml` and `macos.yml` are green on it.

## STOP conditions

- The transcript-diff or frozen log shows any line.
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
