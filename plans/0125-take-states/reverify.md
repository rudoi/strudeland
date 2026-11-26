# 0125 step 0: re-verification on main d38b30e

The issue stands. Only the line numbers moved, because #119 shrank the file.

`src-tauri/src/recording.rs` is 3056 lines: implementation 1-1370, tests 1371-3056. It has 63 tests (62 off macOS: `the_avfoundation_status_reads_without_prompting` is `cfg(target_os = "macos")`), and `cargo test -p strudeland-app --lib recording::` passes them all.

## The state is a set of fields

`Inner` (:304-314) holds:

- `recorder: Option<Recorder>` and `forwarder`, a lifetime pair;
- `stream_denied: bool`;
- `armed: Option<ArmedInput>`;
- `take: Option<Take>`;
- `ended: Option<EndedTake>`;
- `lost_stem: Option<String>`.

`Core` (:317) adds `requesting: AtomicBool`, `platform`, `sink` and `maker`.

These are the reachable combinations, setting aside the recorder lifetime:

| armed | take | ended | meaning |
|---|---|---|---|
| None | None | None | idle; `stream_denied` is true only here |
| Some | None | None | armed |
| Some | Some | None | recording |
| Some | None | Some | ended by the cap, a write error, or a stop whose publish failed while armed; also briefly a lost-device take waiting for its `Failed` |
| None | None | Some | after a device loss, or a failed publish of a lost-device stop |

`take` and `ended` never coexist, because `start` drops `ended` before it sets `take`. `take` without `armed` never happens. `lost_stem` is set only by `ended_by_itself` on `Ended::DeviceLost` (:421) and taken by `failed` (:433), so it bridges a `TakeEnded(DeviceLost)` to its `Failed`. It can outlive `ended` when a stop or disarm runs in between.

## Things each transition must remember

- **`drop_ended`** (:371) publishes an ended take with the fallback sidecar. It has to be called on every way out of the ended state: `arm` :850, `disarm` :904, `start` :943 (after `recorder.start` succeeds) and `finalize_running` :1044, which reload and shutdown both use.
- **The channel fallback** `device.channels == 0 || prefs.channel < device.channels` is written twice: in `resolve_selection` (:514-518) and in `arm` (:826-830, the explicit-device branch).

## Visible behaviour (kept exactly)

Every flow publishes one `recording://state` event, `{state, stem?, error?}`:

| flow | events |
|---|---|
| arm (including a no-op re-arm) | `[armed]` |
| arm, permission denied | `[idle + permissionDenied]` or the restricted text, even while armed, and `armed` stays set |
| arm while a permission request is pending | `busy`, no event |
| arm, device or stream error | no event, `armed` cleared; out-of-range channel gives `other "channel N is out of range for M channels"`; during a take `busy` |
| disarm | `[idle]`, even when already idle; `busy` during a take |
| start | `[recording + stem]`; failures give no event: `busy`, not armed, `projectChanged`/`noProject`, memo errors, `StartTimeout` |
| stop | `[armed\|idle + stem]`; a failed publish adds the error and keeps the take, and a retry repeats it; nothing to stop gives `other "no take in progress"` |
| cap | `[armed + stem + limitReached]`; the later stop gives `[armed + stem]` |
| write error | `[armed + stem + other …]` |
| device lost mid-take | `[idle + stem + deviceLost]`; the later stop gives `[idle + stem]` |
| device lost while armed | `[idle + deviceLost]` |
| backend fault | as a device loss, with an `other "audio backend: …"` error |
| a `Failed` queued after a user disarm | an extra `[idle + error]` |
| reload, quit | no events |

`recording://level` `{peak, rms}` is forwarded without taking the lock.

## Unchanged

- **Command results and errors:** the error kinds and messages (:43-53, `From<RecordError>` with the macOS `DeviceBusy` split, and `CommandError::for_memos`).
- **Sidecars:** fallback versus webview fields, sanitising, `WriteFallback` on stop and `KeepExisting` on a re-stop, `bar1_frame`.
- **The app version:** the sidecar's `app.version` is the app's `CARGO_PKG_VERSION` (:554, 0.1.2, pinned at :2217). Moving `build_sidecar` into another crate would silently change it, so it must be passed in.
- **Files:** `.stem.wav.partial` becomes `stem.wav` plus `stem.json`. A failed publish leaves the partial. `repair_on_open` skips a project whose take is running or ended.

Only 4 tests assert full event sequences (:1904, :1920, :1940, :2025), so event order is barely pinned today. A characterization test of every flow comes first.

## Quirks kept as they are (filed for the user)

- A `recorder.stop()` error in `stop` drops the take (:968-975), leaving the partial to the next open's repair.
- A permission refusal emits `idle` while the recorder stays armed.
- A failed re-arm goes to idle with no event.
- Disarm emits `idle` even when already idle.
- A no-op re-arm emits `armed`.
- A `Failed` queued after a disarm emits an extra `idle + error`.

## Home and coupling

- **Dependencies.** `strudeland-record` (cpal, hound, rtrb, thiserror, mach2) has `strudeland-project` only as a dev-dependency. `strudeland-project` does not depend on record, so there is no cycle. `strudeland-live` depends on record for `clock`. The CLI does not depend on record.
- **What the take logic needs:**
  - from `strudeland-project`: `default_recording_stem`, `recording_partial_path`, `finish_recording`, `write_sidecar`, `recording_entry`, `repair_partials`;
  - from the app: `ProjectState::current_for`, in `start` only, so it is passed as a resolver; the app version; `prefs_path`; and `CommandError`, so the module needs its own error type.
- **Callers.** `Platform`, `Sink` and `RecorderMaker` (:250-269) are used only in `recording.rs`. `lib.rs` calls `on_reload`, `system_state`, `repair_in_background`, the 11 commands and `shutdown`.
- **Threading rules to keep:**
  - The forwarder holds a `Weak` to the core.
  - The forwarder is joined outside the lock in shutdown.
  - Levels are forwarded without the lock.
  - `ensure_permission` runs outside the lock, guarded by `requesting`.
  - The stale-`arm_id` check and the `take_if` path match stay.
  - Poison recovery stays.
  - The lock is held across arm, start (up to 2 s) and stop.
