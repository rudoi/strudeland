# strudeland-render

The render pipeline: a song file to finished audio without the app or a browser. It plans the song
with the headless host (`strudeland-host`), renders the chunks with the engine
(`strudeland-engine`), joins them, sets the loudness and encodes WAV, MP3, Ogg Vorbis or Opus.

## Pipeline and modules

`strudeland_render::render(code, &RenderOptions, progress)` runs the whole thing; `render_audio`
stops after the join and `finish` levels and encodes a joined buffer.

1. **Plan** (`plan`): `Host::plan_render` evaluates the song once and plans every chunk's engine
   job (the length request is the song's clip, a section, the first N bars or an explicit window).
   Material the engine cannot play (samples, zzfx, sbd, bytebeat, kabelsalat) is refused here, with
   the list `s=... keys=... first at bar N`, before anything is rendered.
2. **Render** (`render`): the chunk jobs render in parallel on a scoped worker pool, one engine per
   worker (`--jobs`, default `min(cores, 4)`).
3. **Join** (`join`): each chunk's lookback head and overlap tail are dropped and the contents are
   concatenated sample-exactly (a port of `assembleWindow`; `render:vectors` checks it). The
   window is allocated up front and each chunk is copied in as it finishes and then freed, so the
   render holds the window and the chunks in flight, not every chunk at once.
4. **Level** (`level/`): `loudness` (BS.1770-4 integrated loudness, a port of
   `src/lib/render/loudness.ts`), `true_peak` (4x oversampling with the Annex 2 filter),
   `limiter` (a true-peak lookahead limiter, exponential release of 50 ms); `prepare` checks and
   measures the input and works out one static gain to the target, `Prepared::level` applies it
   and limits at a ceiling (the correction loop prepares once and levels per pass).
5. **Encode** (`encode/`): `wav`, `mp3`, `ogg`, `opus`, `verify` (the decoders) and the true-peak
   correction loop. `output` resolves the default path and writes atomically; `summary` is what a
   render reports (progress events and the JSON summary); `options` the request and its defaults;
   `bench` the timings behind `strudeland-render bench`.

`strudeland render` (in `strudeland-cli`) is this library behind a command. The
`strudeland-render` binary (`render`, `bench`) is the harness's: `render:check --planner batch`
runs it and it is not a `strudeland` subcommand.

## Defaults

| | |
|---|---|
| Loudness | on: -23 LUFS integrated, -1 dBTP true-peak ceiling (EBU R128; `--lufs`, `--true-peak`; `--loudness off` returns the engine output bit for bit). The harness binary defaults to off |
| `wav` | 24-bit PCM with TPDF dither (`--bit-depth 16|24|32f`; 32-bit float is written undithered) |
| `mp3` | CBR 320 kbps; `--mp3-mode vbr` is V0, `--quality 0..9` picks V`N` |
| `ogg` | Vorbis quality 6 (`--quality -1..10`) |
| `opus` | 160 kbps (`--quality` is kbps, 6 to 510); the audio is resampled from 44.1 to 48 kHz |
| Planning timeout | 120 s in the CLI, none in the library |

Without a preset, the render ends at the requested musical boundary.

## Stem sets

Use `strudeland render song.js --stems orbit|voice|instrument`, or select **Stems** in Export
Audio. The usual standard render ranges apply. The default project directory name includes
the song and range, with a `-stems` suffix. The CLI's `--out` takes a new directory path.
Every file is stereo, 44.1 kHz, 32-bit float WAV. The export applies no mastering or
normalization, so samples can exceed integer full scale.

Use `mix.wav` as a reference. At unity gain the numbered stems reproduce it within
floating-point rounding. Orbit stems include their effects. Voice groups use `.label(...)`
and share an unlabeled group when the label is empty or absent. Instrument groups use the
sound name the Rust engine resolves. Both have a shared-effects stem per orbit. That stem
contains the difference between the processed orbit and its direct signal, so a shared
filter can put negative contributions in it. A bus reader's sound belongs to the reader's
group. The catalog includes lookback events, so a group can have a silent file.

`stems.json` identifies the files and records the window, frame count, and sample rate. The
manifest is written last. If a write fails, already-published files are reported and remain
in the chosen directory. An existing destination is refused. A set is limited to 128 stems
and a 512 MiB audio buffer budget. Shorter ranges or fewer parallel workers reduce memory
pressure on large exports.

## Game audio presets

Use `strudeland render song.js --preset PRESET`. The loop presets use the inclusive
`meta.loop` section range or the section marked `loop: true`. They follow any `meta.meterChanges`
inside the loop and exclude the authored ending. Each loop export includes a first pass through the body
and a repeat region of the same length. The one-shot accepts the usual `--section`, `--length`
or `--loops` options.

| Preset | Output |
| --- | --- |
| `intro-loop` | Always two files. Play `<name>-intro.<ext>` once for any intro and the first pass. Then play `<name>-loop.<ext>`, whose repeat marker starts at frame zero. |
| `tagged-loop` | One file with any intro, the first pass and the marked repeat region. Start at frame zero and loop from the marker through EOF. |
| `folded-loop` | One file that starts at the first bar of the loop with fresh effect state. Start at frame zero to hear the first pass, then loop from the marker through EOF. |
| `one-shot` | One file that includes the requested arrangement and then lets triggered notes and effects decay. Trailing zero frames after the requested arrangement are trimmed. |

Intro effects carry into the first pass for `intro-loop` and `tagged-loop`. The repeat region
uses the loop body alone, with its release and effect decay folded back into it. The last
frames of the first pass blend into the corresponding frames of the repeat to match the boundary
waveform even with long tails. In a split export, the game controls the handoff and the opening
has no loop marker. Use WAV for sample-exact playback. A click already present in the source
or imposed by the player or codec can still be audible. MP3 exports warn because playback can
expose encoder delay and padding.

For example, `strudeland render song.js --preset intro-loop --out music.wav` writes
`music-intro.wav` and `music-loop.wav`. Assets are encoded before either is written. Each
write is atomic. If the second write fails, the CLI reports the completed path and the
failed path so the same command can be retried.

WAV uses an embedded forward `smpl` loop. Ogg Vorbis and Opus use `LOOPSTART` and
`LOOPLENGTH` comments, and MP3 stores those names in ID3. The CLI reports the decoded frame
range, sample rate and Godot `loop_offset` in seconds. Opus loop metadata uses 48 kHz.
In Godot, use the WAV metadata detection option for WAV, or enable Loop and enter the
reported offset for Ogg Vorbis or MP3.
[Godot audio import settings](https://docs.godotengine.org/en/4.5/tutorials/assets_pipeline/importing_audio_samples.html)

The loop presets share one static gain across the opening and repeat, capped for the true peak
at the handoff and repeat wrap as well as within the audio.
If any lossy file exceeds the ceiling after decoding, the correction reduces the shared
gain and re-encodes the outputs. `--loudness off` bypasses gain and correction.

## Loudness and true peak

Integrated loudness is BS.1770-4 with the two gates, checked against the TypeScript implementation
within 1e-6 LU and against the EBU Tech 3341 loudness cases. True peak is the ITU-R BS.1770-4
Annex 2 filter (48 taps in four phases of 12). The coefficient table was verified against the ITU
PDF, and the tests check the true-peak cases 15 to 23 of EBU Tech 3341 within its +0.2/-0.4 dB
tolerance, at 44.1 and 48 kHz.

Silence (or less than 400 ms, or below the -70 LUFS gate) has no integrated loudness: no gain is
applied and the render warns.

### The lossy correction loop

A lossy encoder can push the decoded true peak above the ceiling. With loudness on, the encoded
file is decoded again (`symphonia` for MP3 and Vorbis, libopus for Opus) and its true peak measured
over the whole stream. When it is over the ceiling, the limiter's ceiling is lowered by the
overshoot plus 0.1 dB, the original input is leveled again (the same static gain, so the loudness
target holds) and encoded again. That happens at most twice; a peak still over the ceiling after
two corrections is a warning, not an error. Lowering the gain instead would not help a song the
limiter already holds at the ceiling. With loudness off the decoded peak is reported and never
corrected. The summary's `limiterCeilingDbtp` is the ceiling of the final pass.

## Encoders

| Format | Crates | Defaults |
|---|---|---|
| `wav` | own 16/24-bit PCM writer (deterministic TPDF dither); the engine's `write_f32` for 32-bit float | 24-bit |
| `mp3` | `mp3lame-encoder` + `mp3lame-sys` (vendored LAME 4.0) | CBR 320 kbps; VBR V0 to V9 |
| `ogg` | `vorbis_rs` (vendored aoTuV libvorbis and libogg) | quality 6 |
| `opus` | `opus` + `opusic-sys` (vendored libopus), `rubato` (44.1 to 48 kHz), `ogg` (hand-written Ogg Opus muxing) | 160 kbps |

## Determinism

Every format is byte-reproducible for the same song, `meta.seed`, options and crate versions: the
engine renders are deterministic, the chunk joiner is sample-exact whatever the worker count, the
WAV dither generators are seeded per channel, and the Ogg and Opus stream serials are fixed. An
encoder crate bump can change the bytes (and so tests that pin them).

## Building

The C encoders build with the crate: a C compiler for LAME and libvorbis, and `cmake` (3.x or
newer) for libopus. `cargo build -p strudeland-render` works from a clean target once both are
installed (on macOS, `brew install cmake` and the Xcode command line tools). The first build of
`strudeland-host`, which this crate depends on, also downloads a prebuilt V8 (about 40 MB).

The vectors under `tests/vectors/` come from the TypeScript implementations and are written only by
`bun run render:vectors --write`.

## Licences

The crate is AGPL-3.0-or-later. The encoder and decoder libraries it links carry their own
licences, kept verbatim in the crate root (`tests/licence.rs` checks that each is present and
covers a crate in the dependency tree):

| File | Covers |
|---|---|
| `LICENSE-lame` | `mp3lame-encoder`, `mp3lame-sys`, LAME 4.0 (LGPL) |
| `LICENSE-vorbis` | `vorbis_rs`, `aotuv_lancer_vorbis_sys`, `ogg_next_sys` (BSD-3-Clause), aoTuV/Lancer libvorbis, libogg |
| `LICENSE-opus` | `opusic-sys`, libopus (BSD-3-Clause) |
| `LICENSE-opus-rs` | `opus` (MIT or Apache-2.0) |
| `LICENSE-ogg-rs` | `ogg` (BSD-3-Clause) |
| `LICENSE-symphonia` | the `symphonia` crates (MPL-2.0, used unmodified) |
| `LICENSE-rubato` | `rubato`, `realfft`, `audioadapter`, `audioadapter-buffers`, `audioadapter-sample` (MIT or Apache-2.0) |

LAME (LGPL) is statically linked into an AGPL-3.0 binary. The complete corresponding source of the
binary is public and the build instructions above let anyone rebuild and relink it against a
modified LAME, which satisfies the LGPL's relinking requirement; the notices are kept.
