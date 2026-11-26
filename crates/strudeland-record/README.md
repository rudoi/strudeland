# strudeland-record

Microphone capture for voice memos: one input channel written to a mono 32-bit float WAV at the
device's own rate, with level readings and clock anchors (frame index to host time). No Tauri
dependency. The take lifecycle is `takes`; the app wiring (wire types, permission, commands) is
`src-tauri/src/recording.rs`; the sidecar and file naming live in `strudeland-project`.

## Threads

- The audio callback (CoreAudio's thread) only touches atomics and preallocated lock-free `rtrb`
  rings: samples and anchors go in, nothing allocates or blocks.
- The writer thread (while armed) drains the rings, writes the WAV with `hound`, computes levels,
  rewrites the header and flushes about once a second, and enforces the length cap.
- The control thread owns the `Source` and every state transition (`Recorder`: arm, start, stop).

Audio comes through the `Source` seam: `CpalSource` in the app, `testing::FakeSource` in tests, so
no test touches hardware or asks for permission.

## Takes

`takes::Takes` runs the takes over the `Recorder`: one value of `TakeState` under one lock.

- **Idle**: no stream. Left by a successful arm. `stream_denied` records that the last arm's
  stream open was refused (desktops without an in-app prompt learn access from it).
- **Armed**: a stream, no take. Left by start (recording), disarm, reload and quit (idle), and a
  stream failure (idle).
- **Recording**: a take runs. Left by stop, by an ending the recorder reached itself (cap, write
  error, device loss: ended), and by reload and quit (the take is published, then idle).
- **Ended**: a take the recorder finalized, or a stop whose publish failed, kept for the
  webview's stop; it keeps the armed input unless the stream is gone. Left by stop (armed or
  idle), and by arm, start, disarm, reload and quit.

Every way out of `Ended` goes through `leave`, which publishes an ended take that is not yet
published with the fallback sidecar. A stop moves its ended take out first, so only a successful publish leaves
`Ended` without that fallback. `pending_loss_stem` bridges a `TakeEnded(DeviceLost)` and the
`Failed` the recorder sends after it: the stem that `Failed`'s idle event carries. It sits beside
the state, not in it, because a stop or a disarm can run in between.

The lock is held across arm (the recorder's arm), start (up to the start timeout) and stop.
Levels and the permission request run outside it, the request behind a `requesting` flag. The
event forwarder thread holds a `Weak<Takes>`, and shutdown joins it outside the lock. The app
passes its own version in `TakesConfig`, so the sidecar's `app.version` is the app's, not this
crate's.

## File format

Mono, 32-bit float, the device's sample rate, a `WAVE_FORMAT_EXTENSIBLE` header (68 bytes; the data
size sits at byte 64). Because the header is rewritten on every flush the file is a valid WAV up to
the last flush, which is what repairs a crashed take. The cap is two hours, or 1e9 frames above
138 888 Hz (the u32 data-size limit); a take that reaches it ends by itself.

## Timing

Each callback block yields an `Anchor { frame, host_ns }`: the index of its first frame and its
capture time on the clock of `host_now_ns()`. CoreAudio supplies mach time directly. WASAPI supplies QPC time in 100 ns units. ALSA timestamps are stream-relative. After playing each new ALSA stream, the owner samples its clock across advancing periods and maps its event timestamps to process time. Where those samples are unavailable, advancing callbacks estimate the offset from observed host time. A discontinuity or an invalid stamp during a take leaves the WAV playable with no bar-1 grid. `frame_at` interpolates
between anchors (extrapolating at the nominal rate outside them) to find the frame at a host time,
which is how the app computes the memo's `bar1Frame`.

## The capture-timestamp question

cpal's input timestamps give a `callback` instant (`mHostTime`) and a `capture` instant (callback
minus buffer, device latency and safety offset). It is not documented whether `mHostTime` already
marks the buffer's first frame, in which case `capture` counts the buffer twice. `CaptureTime` has
three candidates (`Capture`, `Callback`, `CallbackMinusExtra`); `DEFAULT_CAPTURE_TIME` is
`Capture` and provisional. Measured on one machine, the candidates differ by about 10.7 and 11.7
ms (13.75 ms on a USB device). Only a loopback measurement (speakers into the built-in mic, detect
the click onsets, compare with `bar1Frame`) can pick one; that calibration is a planned follow-up.

The probe prints raw timestamps next to `host_now_ns()` and the candidate shifts. Run it by hand
(the terminal needs microphone access, or every sample reads 0.0):

```sh
cargo run -p strudeland-record --example probe -- [--list] [--device ID] [--seconds N]
```

## Known differences

- Input only, one channel (0-based, chosen in the app), default input config. No resampling.
- Capture accepts f32, i16, i24, i32 and u16 configurations. The writer acknowledges its first samples even on a device without usable clock timestamps.

## Licences

cpal (Apache-2.0), hound (Apache-2.0), rtrb (MIT or Apache-2.0), thiserror, mach2, serde,
serde_json and libc (Linux only) (MIT or Apache-2.0). The crate is AGPL-3.0-or-later.
