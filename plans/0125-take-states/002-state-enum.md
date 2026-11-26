# 002: one state enum, in place

## Goal

Replace `Inner`'s `stream_denied`, `armed`, `take` and `ended` with one `state: TakeState`, and
`lost_stem` with the documented `pending_loss_stem`. Also replace:

- the four `drop_ended` calls, with `leave` and `settle_ended`;
- the two copies of the channel fallback, with `saved_channel`.

All of it stays inside `src-tauri/src/recording.rs`, and every function keeps its signature.
The transcript's expected JSON does not change.

## Prerequisites

- Stage 001 is committed. `T0` (000-prompt) resolves to it.
- Read the proposal's "The state", "`leave` and `saved_channel`" and "Transitions".

## Files

- `src-tauri/src/recording.rs`: the implementation, plus the four tests that read `Inner`'s
  fields, rewritten.
- `src-tauri/src/recording/rig.rs`: one test helper, `armed_id(core) -> Option<u64>`.
- `src-tauri/src/recording/transcript.rs`: no change to the expected block. The driver needs no
  change either, since every function it calls keeps its signature.

## Implementation

1. **The types.** Add `enum TakeState { Idle { stream_denied: bool }, Armed(ArmedInput),
   Recording { armed: ArmedInput, take: Take }, Ended { armed: Option<ArmedInput>, ended:
   EndedTake } }` with the proposal's doc comments.
   - `Inner` becomes `{ recorder, forwarder, state, pending_loss_stem }`, and its `Default`
     gives `Idle { stream_denied: false }`. Write `impl Default` by hand, because the enum has
     no derivable default.
   - Add two private readers on `TakeState`:
     - `armed(&self) -> Option<&ArmedInput>`, which is `Some` for `Armed`, `Recording` and
       `Ended { armed: Some }`;
     - `is_recording(&self) -> bool`.
2. **`leave(inner: &mut Inner, next: TakeState)`.** Use `std::mem::replace`. If the old value is
   `Ended { ended, .. }` with `!ended.published`, call `publish_fallback(&ended.meta,
   &ended.result)` once the new state is in place. The publish runs under the lock, as today.
   - **`settle_ended(inner)`**: if `Ended { armed, .. }`, `leave(inner, armed.map(Armed)
     .unwrap_or(Idle { stream_denied: false }))`. Otherwise do nothing.
   - Delete `drop_ended`.
3. **`saved_channel(device: &DeviceInfo, prefs: &Prefs) -> u16`.** `resolve_selection` and
   `arm`'s `Some(id)` branch both call it. In the branch, `channel.unwrap_or(saved_channel(device,
   &prefs))`.
4. **The verbs**, each a re-expression of today's body (the proposal's Transitions list is the
   spec). Keep every statement's order. In particular:
   - `arm`: the early busy check reads `is_recording()`.
     - Under the lock: busy, then `settle_ended`, `ensure_recorder` and `arm_with_id`.
     - On `PermissionDenied`, `inner.state = Idle { stream_denied: true }`. On other errors,
       `Idle { stream_denied: false }`. Use plain assignment for both, because `settle_ended`
       already ran. On success, `Armed(new)`.
     - The prefs write and `armed_status` stay after the lock.
   - `disarm`:
     - busy if recording;
     - `settle_ended`;
     - `recorder.disarm()`;
     - `inner.state = match state { Idle { stream_denied } => Idle { stream_denied }, _ =>
       Idle { stream_denied: false } }`;
     - emit `idle`.
   - `start`:
     - busy if recording;
     - `let (Some(armed), Some(recorder)) = (inner.state.armed(), &inner.recorder) else { not
       armed }`;
     - then `project_for`, the stem, the path, the timestamp and `recorder.start(&path)?`;
     - clone `armed.info`. The `Recording` state needs the `ArmedInput` itself, so take it out
       by `std::mem::replace` after `recorder.start` succeeded;
     - then `leave(inner, Recording { armed, take })`;
     - emit.

     The `ArmedInput` moves out of `Armed` or `Ended { armed: Some }`, and `leave` publishes
     an unpublished ended take exactly where `drop_ended` did.
   - `stop`:
     - From `Recording`, take it out. With no recorder, put `Armed(armed)` back and return not
       armed, so the take is dropped as today. On a `recorder.stop()` error, set `Armed(armed)`,
       log, and return the error (#155). Otherwise make `Ended { armed: (!device_lost).then_some(armed),
       ended: EndedTake { published: false, .. } }`.
     - Then, if the state is `Ended`, move the `EndedTake` out. Set the state to
       `armed.map(Armed).unwrap_or(Idle { stream_denied: false })`. Compute the event's
       `RecState` from `armed`.
     - `complete_ended`. On Err with `!ended.published`, put back `Ended { armed, ended }`.
       From `Idle` or `Armed`, return no take.
   - `ended_by_itself`:
     - Act only if the state is `Recording` and `take.path == result.path`. This replaces
       `take_if`. Leave every other state untouched.
     - Publish the fallback, set `Ended { armed: Some(armed), ended }`, and emit as today.
       `DeviceLost` sets `pending_loss_stem`.
   - `failed`:
     - `let stem = inner.pending_loss_stem.take();` comes first, then the stale check on
       `state.armed()`.
     - Then clear the armed input: `Armed` goes to `Idle { false }`, `Ended { Some }` to
       `Ended { None }`, and `Idle` and `Ended { None }` stay.
     - `Recording` stays as it is. This case is unreachable; add a comment saying why (the
       proposal's `failed` bullet).
     - Emit.
   - `finalize_running`:
     - `settle_ended`;
     - if `Recording`, take it out, set `Armed(armed)` (the caller clears it next), and stop
       and publish as today;
     - with no recorder, return. Like today, the take is dropped.
   - `on_reload`: return if `Idle`. Otherwise `finalize_running`, `recorder.disarm()` and
     `Idle { stream_denied: false }`.
   - `shutdown`: `finalize_running`. Then `Idle { stream_denied: false }` unless already idle,
     in which case keep the flag. Then take the recorder and forwarder and join them outside the
     lock.
   - `status`, `input_devices`, `repair_on_open`: read through `state.armed()`,
     `Recording { take, .. }` and `Ended { ended, .. }`.
5. **Doc comments.** Say what each variant means, and that `stream_denied` is only meaningful
   in `Idle`. `pending_loss_stem` gets the proposal's bridge comment. Update the module header's
   "The logic lives in plain functions over [`Core`]" to mention `TakeState`.

## Tests

Replace; don't layer. Every test keeps its name and assertions except where noted.

- **Rewritten reads of `Inner`:**
  - `a_stale_failed_after_a_rearm_is_ignored`: `rig.core.lock().armed.as_ref().unwrap().arm_id`
    becomes `armed_id(&rig.core).unwrap()`, through the new `rig.rs` helper over
    `state.armed()`.
  - `repair_runs_in_another_project_during_a_take`: `rig.core.lock().take.is_some()` becomes
    `rig.core.lock().state.is_recording()`.
  - `a_reload_while_idle_is_a_no_op` and `quit_finalizes_the_take_and_joins` read `recorder` and
    `forwarder`, which are unchanged fields. Leave them as they are.
- **New, in `mod tests`:**
  - `leaving_an_unpublished_ended_take_publishes_it`:
    1. Take, block publication, then `stop()` fails. Assert `matches!(state, Ended { .. })`.
    2. Remove the conflict and call `leave(&mut inner, Idle { stream_denied: false })`.
    3. Assert the files are `main-01.json` and `main-01.wav` with the fallback sidecar, and the
       state is idle.
  - `leaving_a_published_ended_take_writes_nothing`:
    1. Hit the cap, so the ended take is published.
    2. Overwrite `main-01.json` with marker bytes.
    3. `settle_ended`: the bytes are unchanged, and the state is `Armed`.
  - `leaving_a_state_that_is_not_ended_publishes_nothing`: from `Armed`,
    `leave(Idle { false })`. The `recordings/` listing is unchanged and empty.
  - `saved_channel_cases`:
    - `channels == 0` keeps any saved channel (an exclusive endpoint);
    - in range keeps it;
    - equal to `channels` gives 0;
    - above gives 0.
- `selection_resolution` is unchanged and still covers the clamp through `resolve_selection`.
- Count: 64 + 4 = 68 tests in `recording::`.

## Validation

- The common gates in [000-prompt.md](000-prompt.md), all `exit 0`, **including the
  transcript-diff gate** (no diff lines).
- `grep -n "drop_ended\|lost_stem\|inner.armed\|inner.take\b\|inner.ended\|stream_denied =" src-tauri/src/recording.rs`
  finds only `stream_denied` inside `TakeState` patterns or constructors, and
  `pending_loss_stem`.
- `grep -c "device.channels == 0 || prefs.channel < device.channels" src-tauri/src/recording.rs`
  is 1.
- **Code behind `cfg`.** None of the edited code is `cfg`'d. `Permission`'s
  `cfg_attr(not(macos), allow(dead_code))` attributes are untouched in this stage. The
  `permission` module and `SystemPlatform` are untouched.

## STOP conditions

- The transcript-diff log shows any line.
- A transition would need an extra event, a different order or a new wait to keep a test green.
- A reachable state cannot be represented. The one known unreachable case is a matching
  `Failed` while `Recording`; report any other.
- A quirk (#155) would disappear.

## Commit message

```
refactor(app): hold the recorder's take in one state enum

Idle, armed, recording and ended become one enum, so the combinations the
fields allowed but nothing reached cannot be built. One leave() publishes an
ended take on the way out of that state, replacing four drop_ended calls,
and saved_channel() is the one channel fallback. Events, results and files
are unchanged.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
