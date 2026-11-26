# 001: the transcript of every take flow

## Goal

Pin what the recorder does today, before any edit. That means:

- every `recording://state` event, with its order;
- every command result, `Ok` or `CommandError`;
- the `recordings/` listing;
- the key sidecar fields;

for every flow in reverify's event table, plus the failed-publish retry and the ended-take
settles. Everything is asserted against one inline JSON constant that stages 002 and 003 may not
change.

This stage changes no production code. The existing test fakes move to a shared rig file, so
the transcript and the existing tests use the same ones.

## Prerequisites

- On `llm/andrew/strudeland/0125-take-states` at `d38b30e`, with only `plans/` untracked.
- `cargo test -p strudeland-app --lib recording::` passes (63 tests on macOS) before you
  start. Log it as `$S/0125-001-before.log`.

## Files

- `src-tauri/src/recording.rs`: add `#[cfg(test)] mod rig;` and `#[cfg(test)] mod transcript;`
  next to `mod tests`. Remove the moved fakes from `mod tests`, which now does
  `use super::rig::*;`.
- `src-tauri/src/recording/rig.rs` (new): the fakes and the rig, moved verbatim.
- `src-tauri/src/recording/transcript.rs` (new): the driver, the flows and the expected
  constant.

## Implementation

### 1. Move the fakes to `rig.rs`

Cut these from `mod tests` (`recording.rs:1383-1662` and `:2199-2201`, `:2328-2333`) and paste
them into `rig.rs` unchanged except for visibility:

- `RATE`, `WAIT`, `T0`;
- `device`, `mic`, `usb`;
- `FakePlatform` with its `Platform` impl;
- `FakeSink` with `states`, `last` and `wait`;
- `Rig`, `rig_with`, `rig`;
- `start_fields`, `stop_fields`;
- `impl Rig`: `prefs`, `project`, `recordings_dir`, `files`, `next_handle`, `no_new_handle`,
  `arm`, `start`, `start_with`, `push`, `take`;
- `sidecar_of`, `block_publication`.

Make each item, and every field the tests read, `pub(super)`. That makes them visible to
`recording`, `recording::tests` and `recording::transcript`. `rig.rs` starts with
`use super::*;` plus the imports the moved code needs, and the `tests` module keeps the imports
only it uses. `start_fields_json` and `stale_partial` stay in `tests`.

No assertion changes, and no test is renamed. `cargo test -p strudeland-app --lib recording::`
still runs 63 tests on macOS, all passing.

### 2. The driver (`transcript.rs`)

The file has three parts, in this order:

1. **The adapters**: `Script`, its step recorders and one method per verb or trigger. These are
   the only lines later stages may change, because they call the functions under test.
2. **The frozen region**, between `// --- frozen: begin ---` and `// --- frozen: end ---`:
   `normalize`, the restricted-text literals, the flow functions and the test function. The
   flows call only `Script` methods and `Rig` helpers (`rig.platform`'s fields,
   `block_publication`, `rig.project()`), never `rig.core` or a `recording.rs` function, so a
   later stage changes the adapters and `rig.rs`, never a flow.
3. **The expected block**, between `// --- expected: begin ---` and `// --- expected: end ---`.

From stage 002 on, the frozen region and the expected block are both diffed against the last
stage-001 commit (000-prompt's `T0` gate).

```rust
use super::rig::*;
use super::*;
use serde_json::{Value, json};

struct Script { rig: Rig, steps: Vec<Value>, seen: usize }
```

- `Script::new(rig)`.
- `call(&mut self, label, result: Result<impl Serialize, CommandError>)` records one step:

  ```json
  { "do": label, "result": {"ok": …} | {"err": {"kind", "message"}}, "events": [new StateEvents], "files": rig.files() }
  ```

  `events` holds `serde_json::to_value` of each `StateEvent` the sink received since the
  previous step (`self.seen` indexes `rig.sink.states()`). `files` is null when `recordings/`
  is not a directory (the `start-memo-error` flow). The driver reads the directory itself
  rather than calling `Rig::files`, which unwraps.
- `act(&mut self, label, f: impl FnOnce(&Rig))` is a step with `"result": null`, for actions on
  the rig and its fakes that trigger nothing asynchronous: blocking or unblocking publication, a
  permission or device change, a prefs read, a stale partial. The synchronous verbs with no
  result (`reload`, `quit`, `inject_failed`, `inject_ended`) are adapters that record the same
  `null`-result step.
- **A step that triggers async work snapshots its events only after its own wait.** The
  recorder's control thread sends `TakeEnded` and then `Failed` after a fault
  (`recorder.rs:461-474`), the cap's `TakeEnded` comes from the writer, and a gated permission
  request finishes on another thread. A trigger and its wait are therefore one step, and only
  these helpers make such a step:
  - `fault_and_wait(label, handle, kind, message)`: `handle.fault(kind, message)`, then
    `rig.sink.wait` for the first event with an error after `self.seen`, then the snapshot;
  - `push_and_wait_error(label, handle, from, to)`: the push (the cap), the same wait, then the
    snapshot;
  - `release_and_join(label, gate_tx, join_handle)`: release the gate, join the spawned `arm`,
    then the snapshot, with the joined result as the step's result. The first arm's `armed`
    event lands in this step.

  No `act` performs a trigger, and no step sleeps. `wait` scans only events after `self.seen`,
  so an earlier flow step's error cannot satisfy it.
- `status(&mut self)` is `call("status", Ok(status(&rig.core, &rig.prefs())))`.
- Each verb has one adapter method (`arm`, `disarm`, `start`, `stop`, `devices`, `status`,
  `reload`, `quit`, `inject_failed`, `inject_ended`, `repair`, `sidecar`). Each one calls the
  `recording.rs` function and passes the result to `call` with `.map_err(CommandError::from)`
  where the error type is not already `CommandError`, so stage 003 changes the call and keeps
  the step.
- `entry(value)` summarizes a `RecordingEntry` or a `Sidecar` to its key fields, and is used in
  `call` for stops and in a `sidecar(stem)` step that reads `rig.project().read_sidecar(stem)`:
  - `stem`, `frames`, `sampleRate`, `duration`, `hasSidecar`, `sidecarError`, `size`;
  - `sidecar`: `schema`, `app.version`, `input`, `output`, `tempo`, `backing`, `xruns`,
    `recovered`;
  - `timing`: `bar1Frame`, `bar1HostNs`, `clockOffsetNs`, `outputStamps`, `anchors.len()` as
    `anchorCount`, and `anchors[0]` as `firstAnchor`.
- `normalize(value, paths)` runs over every recorded value and replaces only these:
  - **paths**, in this order, wherever they appear inside a string:
    1. every project root the flow opened, as `root().to_string_lossy()`: `<root>` for alpha,
       `<root:beta>` for beta. `Project::create` canonicalizes (`project.rs:403`), so on macOS
       the root is `/private/var/folders/…` while `tmp.path()` is `/var/folders/…`, and on
       Windows it is the verbatim `\\?\C:\…` form;
    2. the canonical temp dir (`tmp.path().canonicalize()`): `<tmp>`;
    3. the raw temp dir (`tmp.path()`): `<tmp>`;
    4. then, only in a string where one of these matched, every `\` becomes `/`, so a
       Windows path suffix reads as on Unix;
  - a non-null `elapsedSeconds`: `"<elapsed>"`;
  - `modifiedMs`: `"<mtime>"`;
  - `createdAt`: `"<created>"`;
  - `app.version` equal to `env!("CARGO_PKG_VERSION")`: `"<app version>"`. Any other value is
    kept, so a wrong version shows as a diff;
  - the restricted text: `"<restricted>"`. The transcript writes the two texts itself as
    `cfg`'d literals (`"microphone access is restricted on this mac"` under
    `target_os = "macos"`, `"microphone access is restricted by the system"` otherwise), not
    through `recording.rs`'s `RESTRICTED`. A production text that drifts then shows as a diff,
    and stage 003 can delete the const without touching the frozen region;
  - in the `write-error` flow only, the entry's `frames`, `duration` and `size`: `"<live>"`
    (see Decisions). If Windows CI shows a different `files` listing in this flow only, that
    flow's `files` are normalized too (a sorted set of the names that exist on every OS); a
    difference in any event or result there is a STOP.
- `inject_failed` calls `failed(&rig.core, arm_id, error)` directly, and `inject_ended` calls
  `ended_by_itself(&rig.core, result)`. Both are private functions of `recording.rs`, reachable
  as `super::`, and both run synchronously on the test thread, so they are plain steps. Arm ids
  are the recorder's, sequential from 1 per rig, so a flow knows them by counting its stream
  opens. A no-op re-arm keeps its id.

### 3. The flows

Each flow is a fresh rig unless stated, and the transcript is
`{ "<flow>": [steps…], … }`. Write the steps exactly as listed. A flow that needs a variant
rig (limits, no prompt) builds it first.

| flow | steps |
|---|---|
| `arm-prompt-granted` | permission `notDetermined`, answer `authorized`; `arm()`; `status` |
| `arm-prompt-denied` | answer `denied`; `arm()`; `status` |
| `arm-denied` | permission `denied`; `arm()` |
| `arm-restricted` | permission `restricted`; `arm()` |
| `arm-refused-while-armed` | `arm()`; set permission `denied`; `arm()`; `status` (stays `armed`, #155) |
| `arm-while-request-pending` | `notDetermined` with the gate; spawn `arm()`; wait for `requests == 1` (records nothing); `arm()` (busy, no event); `release_and_join` (the first arm's result and its `armed` event) |
| `no-prompt-stream-decides` | `has_prompt` false, `notDetermined`; `status`; `arm_failure = PermissionDenied`; `arm()`; `status` (`denied`); `arm()`; `status` |
| `arm-no-device` | devices cleared; `arm()`; `arm("fake:gone")` |
| `arm-absent-device` | `arm("fake:gone")` |
| `arm-channel-out-of-range` | `arm("fake:usb", 4)` |
| `rearm` | `arm()`; `arm()` (no-op, emits `armed`, #155); `arm("fake:usb", 2)`; `status` |
| `rearm-fails-to-idle` | `arm()`; `arm_failure = Backend("refused")`; `arm("fake:usb")` (no event, #155); `status` |
| `exclusive-input` | the body of `a_saved_exclusive_input_keeps_its_identity_and_armed_channel_count` as steps, `devices()` included |
| `prefs` | the body of `arm_writes_prefs_only_when_it_carried_an_argument`, with an `act` after each arm that records the prefs file text (or null) in the step |
| `disarm` | `arm()`; `disarm()`; `disarm()` (emits `idle` again, #155) |
| `busy-during-take` | take; `arm()`; `disarm()`; `start()` |
| `start-names` | `arm()`; start `main` (push the first block as `Rig::start` does); `status`; `stop()`; start `main`; `stop()`; start none; `stop()`; start `a/b`; `stop()` |
| `start-not-armed` | `start()` |
| `start-stale-root` | `arm()`; start with root + `-old`; `status` |
| `start-no-project` | `arm()`; start against `ProjectState::default()` |
| `start-memo-error` | `arm()`; replace `recordings/` with a file; `start()` |
| `start-timeout` | limits `start_timeout: 100ms`; `arm()`; `start()` without pushing; `status` |
| `stop-with-fields` | take; `stop(fields bar1 "1500000000")`; `status` |
| `stop-without-fields` | take; `stop()` |
| `stop-invalid-fields` | take; `stop(fields with meter "four")` |
| `stop-bar1-outside` | take; `stop(fields bar1 "500000000")` |
| `stop-nothing` | `stop()`; take; `stop()`; `stop()` |
| `stop-other-project` | take; open `beta`; `stop()`; record `beta`'s `recordings()` as a step |
| `invalid-start-tempo` | the body of `an_invalid_start_tempo_still_saves_the_memo` as steps |
| `publish-retry` | take; block publication; `stop(fields)`; `stop()` (same error again); remove the conflict; `stop(fields)`; `stop()` |
| `settle-by-arm` | take; block; `stop()`; remove the conflict; `arm()`; `sidecar("main-01")` |
| `settle-by-start` | take; block; `stop()`; remove the conflict; start `main`; `sidecar("main-01")`; `stop()` |
| `settle-by-disarm` | take; block; `stop()`; remove; `disarm()`; `sidecar("main-01")` |
| `settle-by-reload` | take; block; `stop()`; remove; reload; `sidecar("main-01")`; `status` |
| `settle-by-quit` | take; block; `stop()`; remove; quit; `sidecar("main-01")` |
| `cap` | limits `max_frames: 100`; `arm()`; start `main`; `push_and_wait_error` 200 frames at `T0 + 10ms`; `sidecar("main-01")`; `status`; `stop(fields bar1 "1050000000")`; `stop()` |
| `cap-then-invalid-fields` | as `cap` to the push; `stop(fields with meter "four")` (keeps the fallback); `sidecar("main-01")` |
| `cap-then-loss` | as `cap` to the push; `fault_and_wait` `DeviceLost "unplugged"` (`[idle + deviceLost]`, no stem: the take already ended, so no `TakeEnded` sets `pending_loss_stem`); `status`; `stop()` (`[idle + stem]`) |
| `cap-then-start` | as `cap` to the push; start `main` without a stop (`main-02`; the published cap take is settled without a write); `sidecar("main-01")`; `stop()`; `sidecar("main-02")` |
| `write-error` | take; `inject_ended` with `ended: WriteError("disk full")`, `path` the take's partial, `frames 1000`, the take's rate and channel, no anchors, 0 xruns; `status`; `stop()`; quit. Quit is the last step: the real recorder's take is still running, so nothing starts after the injection but the quit that ends it |
| `ended-after-stop` | take; `stop()`; `inject_ended` with `ended: Cap` for the stopped take's path (the cap-versus-stop race: `take_if` finds no take, no event, files unchanged); `status` |
| `ended-wrong-path` | take; `inject_ended` with `ended: Cap` for another path in `recordings/` (no event, the take runs on); `status` (`recording`); `stop()` (`[armed + stem]`) |
| `device-lost-take` | take; `fault_and_wait` `DeviceLost "unplugged"`; `sidecar("main-01")`; `status`; `stop(fields bar1 "1250000000")`; `stop()`; `arm()` |
| `device-lost-take-blocked` | take; block; `fault_and_wait`; `stop()` (fails, `idle + stem + error`); remove; `stop()`; `stop()` |
| `device-lost-armed` | `arm()`; `fault_and_wait` `DeviceLost`; `status` |
| `backend-fault-take` | take; `fault_and_wait` `Other "x"`; `stop()` |
| `backend-fault-armed` | `arm()`; `fault_and_wait` `Other "x"` |
| `failed-after-disarm` | `arm()`; `disarm()`; `inject_failed { arm_id: 1, DeviceLost("late") }` (extra `idle + error`, #155) |
| `stale-failed` | `arm()`; `arm("fake:usb")` (id 2); `inject_failed { 1 }`; `status`; `inject_failed { 2 }`; `status` |
| `arm-after-loss` | take; `fault_and_wait`; `arm()`; `stop()` |
| `start-after-loss` | take; `fault_and_wait` `DeviceLost`; `start()` (`other "the recorder is not armed"`, no event; the state stays `Ended { armed: None }`); `status` (`idle`); `stop()` (`[idle + stem]`) |
| `reload-idle` | reload; `arm()`; reload; `status` |
| `reload-take` | take; reload; `status`; `stop()`; `sidecar("main-01")`; `arm()` |
| `quit-take` | take; quit; `sidecar("main-01")`; quit |
| `repair` | take; stale partial `memo-09`; `repair_on_open` (`skipped`); `fault_and_wait`; repair (`skipped`); `stop()`; repair (`repaired`, `discarded: [".memo-09.wav.partial"]`) |

`repair_on_open` steps record `"skipped"`, `{"repaired": <RepairReport>}` or
`{"failed": message}`. A take is `Rig::take`: `arm()`, start `main` and push frames 10 to 1000.
Log each of its three parts as its own step, so each start's event is pinned.

### 4. Capture and review

1. Write the adapters, then the frozen region, then `const EXPECTED: &str = r#"{}"#;`
   between the markers:

   ```rust
   // --- frozen: begin ---
   fn normalize(…) { … }
   fn flows(…) { … }        // and every flow function
   #[test]
   fn transcript_of_every_take_flow() { … }
   // --- frozen: end ---

   // --- expected: begin ---
   const EXPECTED: &str = r#"…"#;
   // --- expected: end ---
   ```

2. When `STRUDELAND_PRINT_TRANSCRIPT=1`, the test prints
   `serde_json::to_string_pretty(&actual)` before asserting. Run
   `STRUDELAND_PRINT_TRANSCRIPT=1 cargo test -p strudeland-app --lib recording::transcript -- --nocapture > "$S/0125-001-capture.log" 2>&1`.
3. **Review it against reverify's event table, row by row,** before pasting:
   - arm gives `[armed]`, and the permission refusals give `[idle + permissionDenied]`;
   - stop gives `[armed|idle + stem]`;
   - cap gives `[armed + stem + limitReached]`;
   - a device loss gives `[idle + stem + deviceLost]`;
   - the backend fault gives `audio backend: x`;
   - reload and quit give no events.

   Check every flow's files against reverify's "Files" line. If a recorded event, result or file
   disagrees with the table or the existing tests' assertions, **STOP and report**: either the
   table or the code is wrong, and this stage must not decide which.
4. Paste the pretty JSON into `EXPECTED`. The test asserts
   `assert_eq!(actual, serde_json::from_str::<Value>(EXPECTED).unwrap())`, comparing values,
   not text.
5. Run the whole `recording::` module 20 times, in parallel as cargo runs it by default, so
   the transcript is shown stable beside the other tests' threads:
   `for i in $(seq 20); do cargo test -p strudeland-app --lib recording:: -q || echo "FAIL $i"; done > "$S/0125-001-repeat.log" 2>&1`.
   The log must have no `FAIL`. A flaky flow means an async trigger is recorded outside its
   wait helper (`fault_and_wait`, `push_and_wait_error`, `release_and_join`). Fix the driver,
   never by sleeping. If it cannot be made deterministic, STOP and report.
6. Commit (the message below), push the branch and open a **draft** PR against `main`
   (`gh pr create --draft`). `native.yml` (Windows x64, Ubuntu 24.04) and `macos.yml` run only
   on `pull_request` and on pushes to `main`, so a branch push alone runs neither. Wait until
   both workflows are green on the 001 commit before stage 002 starts. That is the first run
   of the transcript on Windows and Linux: the paths, the restricted literal and the
   `write-error` flow's files. A red run on another OS is fixed in the adapters or `normalize`
   in a new commit on top (never a force-push), and only for a normalisation gap; an event or
   result that differs by OS is a STOP. A fix commit on top is still part of stage 001. 000-prompt's `T0` is
   the last commit that touched `transcript.rs`, so it is the green 001 head, and the frozen
   region and the expected block are final from then on.

## Validation

- `cargo test -p strudeland-app --lib recording::` runs 64 tests on macOS: the 63 existing
  ones, unchanged, plus `transcript_of_every_take_flow` (63 and 62 elsewhere, since
  `the_avfoundation_status_reads_without_prompting` is `cfg(target_os = "macos")`). The macOS
  count is the one logged.
- The repeat log has no `FAIL`.
- `native.yml` and `macos.yml` are green on the draft PR's head, the 001 commit or a 001 fix on
  top of it.
- The common gates in [000-prompt.md](000-prompt.md), all `exit 0`. The transcript-diff and
  frozen gates start with stage 002.
- `git diff --stat d38b30e` shows only `src-tauri/src/recording.rs`,
  `src-tauri/src/recording/rig.rs` and `src-tauri/src/recording/transcript.rs` (plus `plans/`).
- **Code behind `cfg`.** The restricted text is `cfg`'d by OS as two literals inside the frozen
  region, so the expected JSON is the same on every OS and does not depend on `RESTRICTED`. `DeviceBusy`'s kind differs by OS, so no flow uses it.
  The moved `the_avfoundation_status_reads_without_prompting` test stays in `tests` with its
  `#[cfg(target_os = "macos")]`. No item moved to `rig.rs` is `cfg`'d.

## STOP conditions

- A recorded flow disagrees with reverify's table or an existing test's assertion.
- A flow cannot be made deterministic without a sleep.
- Windows or Linux CI shows an event or result that differs from macOS.
- Any production line of `recording.rs` would change. Only `mod` declarations and the test
  module's imports may.

## Commit message

```
test(app): pin every take flow's events, results and files

Characterization transcript of the recorder: each flow's state events,
command results, recordings listing and key sidecar fields, asserted against
one inline JSON value. The test fakes move to a shared rig module.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
