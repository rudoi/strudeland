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
- `cargo test -p strudeland-app --lib recording::` passes (63 tests) before you start. Log it as
  `$S/0125-001-before.log`.

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
still runs 63 tests, all passing.

### 2. The driver (`transcript.rs`)

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
- `act(&mut self, label, f: impl FnOnce(&Rig))` is a step with `"result": null`. It is for
  faults, injections, reload, quit, pushes and waits.
- `status(&mut self)` is `call("status", Ok(status(&rig.core, &rig.prefs())))`.
- `entry(value)` summarizes a `RecordingEntry` or a `Sidecar` to its key fields, and is used in
  `call` for stops and in a `sidecar(stem)` step that reads `rig.project().read_sidecar(stem)`:
  - `stem`, `frames`, `sampleRate`, `duration`, `hasSidecar`, `sidecarError`, `size`;
  - `sidecar`: `schema`, `app.version`, `input`, `output`, `tempo`, `backing`, `xruns`,
    `recovered`;
  - `timing`: `bar1Frame`, `bar1HostNs`, `clockOffsetNs`, `outputStamps`, `anchors.len()` as
    `anchorCount`, and `anchors[0]` as `firstAnchor`.
- `wait_error(&mut self, label)` is an `act` that calls `rig.sink.wait(|e| e.error.is_some())`
  for the first event with an error after `self.seen`. Use it for every async event (a cap, a
  fault). No other step waits, and no step sleeps.
- `normalize(value, tmp)` runs over every recorded value and replaces only these:
  - the temp dir's path string, wherever it appears inside a string: `<tmp>`;
  - a non-null `elapsedSeconds`: `"<elapsed>"`;
  - `modifiedMs`: `"<mtime>"`;
  - `createdAt`: `"<created>"`;
  - `app.version` equal to `env!("CARGO_PKG_VERSION")`: `"<app version>"`. Any other value is
    kept, so a wrong version shows as a diff;
  - the platform's restricted text (the `RESTRICTED` const): `"<restricted>"`;
  - in the `write-error` flow only, the entry's `frames`, `duration` and `size`: `"<live>"`
    (see Decisions).
- `injected Failed` steps call `failed(&rig.core, arm_id, error)` directly, and the injected
  `TakeEnded` calls `ended_by_itself(&rig.core, result)`. Both are private functions of
  `recording.rs`, reachable as `super::`. Arm ids are the recorder's, sequential from 1 per
  rig, so a flow knows them by counting its stream opens. A no-op re-arm keeps its id.

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
| `arm-while-request-pending` | `notDetermined` with the gate; spawn `arm()`; wait for `requests == 1`; `arm()` (busy); release; join the first `arm` as a step |
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
| `cap` | limits `max_frames: 100`; `arm()`; start `main`; push 200 frames at `T0 + 10ms`; `wait_error`; `sidecar("main-01")`; `status`; `stop(fields bar1 "1050000000")`; `stop()` |
| `cap-then-invalid-fields` | as `cap` to the wait; `stop(fields with meter "four")` (keeps the fallback); `sidecar("main-01")` |
| `write-error` | take; inject `TakeEnded` with `ended: WriteError("disk full")`, `path` the take's partial, `frames 1000`, the take's rate and channel, no anchors, 0 xruns; `status`; `stop()`; quit |
| `device-lost-take` | take; fault `DeviceLost "unplugged"`; `wait_error`; `sidecar("main-01")`; `status`; `stop(fields bar1 "1250000000")`; `stop()`; `arm()` |
| `device-lost-take-blocked` | take; block; fault; `wait_error`; `stop()` (fails, `idle + stem + error`); remove; `stop()`; `stop()` |
| `device-lost-armed` | `arm()`; fault `DeviceLost`; `wait_error`; `status` |
| `backend-fault-take` | take; fault `Other "x"`; `wait_error`; `stop()` |
| `backend-fault-armed` | `arm()`; fault `Other "x"`; `wait_error` |
| `failed-after-disarm` | `arm()`; `disarm()`; inject `Failed { arm_id: 1, DeviceLost("late") }` (extra `idle + error`, #155) |
| `stale-failed` | `arm()`; `arm("fake:usb")` (id 2); inject `Failed { 1 }`; `status`; inject `Failed { 2 }`; `status` |
| `arm-after-loss` | take; fault; `wait_error`; `arm()`; `stop()` |
| `reload-idle` | reload; `arm()`; reload; `status` |
| `reload-take` | take; reload; `status`; `stop()`; `sidecar("main-01")`; `arm()` |
| `quit-take` | take; quit; `sidecar("main-01")`; quit |
| `repair` | take; stale partial `memo-09`; `repair_on_open` (`skipped`); fault; `wait_error`; repair (`skipped`); `stop()`; repair (`repaired`, `discarded: [".memo-09.wav.partial"]`) |

`repair_on_open` steps record `"skipped"`, `{"repaired": <RepairReport>}` or
`{"failed": message}`. A take is `Rig::take`: `arm()`, start `main` and push frames 10 to 1000.
Log each of its three parts as its own step, so each start's event is pinned.

### 4. Capture and review

1. Write the driver and flows with `const EXPECTED: &str = r#"{}"#;` between the markers:

   ```rust
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
5. Run the transcript 20 times to show it is stable:
   `for i in $(seq 20); do cargo test -p strudeland-app --lib recording::transcript -q || echo "FAIL $i"; done > "$S/0125-001-repeat.log" 2>&1`.
   The log must have no `FAIL`. A flaky flow means a step is missing a `wait_error` for an async
   event. Fix the driver, never by sleeping. If it cannot be made deterministic, STOP and
   report.

## Validation

- `cargo test -p strudeland-app --lib recording::` runs 64 tests: the 63 existing ones,
  unchanged, plus `transcript_of_every_take_flow`.
- The repeat log has no `FAIL`.
- The common gates in [000-prompt.md](000-prompt.md), all `exit 0`. The transcript-diff gate
  starts with stage 002.
- `git diff --stat d38b30e` shows only `src-tauri/src/recording.rs`,
  `src-tauri/src/recording/rig.rs` and `src-tauri/src/recording/transcript.rs` (plus `plans/`).
- **Code behind `cfg`.** `RESTRICTED` is `cfg`'d by OS; the normalizer reads the const, so the
  expected JSON is the same on every OS. `DeviceBusy`'s kind differs by OS, so no flow uses it.
  The moved `the_avfoundation_status_reads_without_prompting` test stays in `tests` with its
  `#[cfg(target_os = "macos")]`. No item moved to `rig.rs` is `cfg`'d.

## STOP conditions

- A recorded flow disagrees with reverify's table or an existing test's assertion.
- A flow cannot be made deterministic without a sleep.
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
