# strudeland-record

Microphone capture for voice memos: one input channel written to a mono 32-bit float WAV at the
device's own rate, with level readings and clock anchors (frame index to mach host time). No Tauri
dependency. The app wiring is `src-tauri/src/recording.rs`; the sidecar and file naming live in
`strudeland-project`.

## Threads

- The audio callback (CoreAudio's thread) only touches atomics and preallocated lock-free `rtrb`
  rings: samples and anchors go in, nothing allocates or blocks.
- The writer thread (while armed) drains the rings, writes the WAV with `hound`, computes levels,
  rewrites the header and flushes about once a second, and enforces the length cap.
- The control thread owns the `Source` and every state transition (`Recorder`: arm, start, stop).

Audio comes through the `Source` seam: `CpalSource` in the app, `testing::FakeSource` in tests, so
no test touches hardware or asks for permission.

## File format

Mono, 32-bit float, the device's sample rate, a `WAVE_FORMAT_EXTENSIBLE` header (68 bytes; the data
size sits at byte 64). Because the header is rewritten on every flush the file is a valid WAV up to
the last flush, which is what repairs a crashed take. The cap is two hours, or 1e9 frames above
138 888 Hz (the u32 data-size limit); a take that reaches it ends by itself.

## Timing

Each callback block yields an `Anchor { frame, host_ns }`: the index of its first frame and its
capture time on the mach host clock, the same clock as `host_now_ns()`. `frame_at` interpolates
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
- macOS only for host time (`mach2`); other platforms are not supported.

## Licences

cpal (Apache-2.0), hound (Apache-2.0), rtrb (MIT or Apache-2.0), thiserror and mach2 (MIT or
Apache-2.0). The crate is AGPL-3.0-or-later.
