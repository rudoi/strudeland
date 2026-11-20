# Loop export modes

## Goal

Give `strudeland render` four per-render presets for game audio. An export can carry loop
points in a single file, deliver an intro and loop separately, fold a loop's decay into its
beginning, or let a one-shot decay to silence. The song file remains the source of the arrangement.

## Scope

Propose `--preset tagged-loop|intro-loop|folded-loop|one-shot`, with the existing render behavior
as the default when the flag is omitted.

| Preset | Musical content | Output |
| --- | --- | --- |
| `tagged-loop` | Intro and one pass of the designated loop section | One file with forward loop points. Playback reaches EOF at the loop end. |
| `intro-loop` | The same range, split at the loop start | An intro file when the intro is nonempty and a body file that can loop from frame zero. |
| `folded-loop` | One isolated pass of the designated loop section plus its decay | One body file. Decay after the last bar is added back into the loop. |
| `one-shot` | The requested clip, section or length, followed by post-roll | One file without loop markers. Written ending sections play as part of a full clip. |

The three loop presets use the single `loop: true` section in the evaluated song structure.
Require that section and make `--section`, `--length` and `--loops` mutually exclusive with
these presets. The one-shot preset keeps the existing length flags, including `--loops` for a
clip. A composed ending belongs to the one-shot clip and is outside the three loop preset
windows. This task covers the offline CLI and the shared renderer. The desktop export UI,
saved presets, live playback, and game project integration are outside its scope.

## Architecture

- Resolve preset windows where the song is already evaluated, in
  [`planRender`](../../src/lib/render/job-plan.ts). Use its `SongStructure`, including the
  structure provided by `song(meta, parts)`. Extend the typed
  [Rust planning request](../../crates/strudeland-host/src/request.rs) and keep the existing
  batch path for callers without a preset.
- Keep the existing host-to-engine job path, unsupported-feature check, workers and joiner.
  Carry export boundaries and any extra frame count through the plan as data. Shape the
  resulting PCM in [`strudeland-render`](../../crates/strudeland-render/src/lib.rs) before
  leveling and encoding. Keep the single-file `render`, `render_audio` and `finish` callers
  working, including the harness and the encoder benchmark.
- Reuse the CLI's render flag validation, result formatting and filesystem entry points in
  [`render.rs`](../../crates/strudeland-cli/src/render.rs) and
  [`run_render`](../../crates/strudeland-cli/src/run.rs). Keep the CLI output shaped around
  the resulting files and expose the loop range and Godot hint when applicable.

## Technical Design

### Arrangement and post-roll

Derive positions from absolute bar boundaries with
[`window_frames`](../../crates/strudeland-render/src/join.rs). Represent a loop by its
zero-based start frame and exclusive end in the exported file. A separate loop starts at
frame zero. Convert to the output codec's frame rate at the encoder boundary.

Keep the last scheduled bar distinct from the last rendered frame. Query only the musical
span and continue processing its scheduled voices and effects through the post-roll. In
particular, extending the schedule through a non-structured pattern would trigger its next
cycle, and extending through a structured clip could play an authored ending. Retain the
last chunk's DSP state through its post-roll. Keep the existing memory limits for the
scratch render and joined buffer, and reject out-of-range durations before allocation.
Extend the final job's frame count so the
[`Engine::run` render loop](../../crates/strudeland-engine/src/render.rs) keeps processing
after its final pause. Keep the post-roll from that chunk even when workers complete
out of order. Its first frame is the chunk's lookback head length plus the chunk's
sample-exact content length.

Use the existing [lookback model](../../src/lib/render/lookback.ts) as the basis for a
decay estimate that accounts for voice releases, reverb, feedback delay and bus dependencies.
Define a documented decay cutoff, including what happens for silence and long feedback.
Include the first delayed echo even at zero feedback and account for the engine's effective
hap duration and reverb impulse response when deriving that cutoff.
The planner can subtract elapsed time since the relevant events and round the remaining
post-roll up to frames. For folded loops, query the marked body at its original musical
position with no haps or DSP history from the intro. Add each post-roll frame into the
corresponding body frame modulo body length. The output therefore has exactly one loop's
frames, including when the tail spans several repetitions. Fold decay in float PCM before
any gain or quantization. An authored discontinuity and discontinuities introduced by lossy
encoding still depend on the source audio and decoder.

### Leveling and encoding

Use a shared level pass over the contiguous intro and loop before splitting so the files
hand off at the same gain. For a folded loop, prefer a constant gain that meets the
true-peak ceiling at the wrap as well as inside the file. A linear look-ahead limiter can
change gain differently at the two ends. Keep the existing decoded-peak verification
for lossy formats, including re-encodes. When leveling a pair, apply any level adjustment
from a correction consistently to both files. Keep the current loudness-off behavior.

- WAV writes a forward `smpl` loop with an inclusive end frame. It expresses the same range
  as `LOOPSTART` and `LOOPLENGTH` in a WAV-native chunk. Preserve the sample payload and
  check RIFF sizes and chunk padding. [RIFF specification](https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/WAVE/Docs/RIFFNEW.pdf)
- Ogg Vorbis writes `LOOPSTART` and `LOOPLENGTH` as comments at the PCM sample rate.
  [Vorbis specification](https://xiph.org/vorbis/doc/Vorbis_I_spec.pdf)
- Opus writes the same names in `OpusTags`. Convert start and end independently into
  decoded 48 kHz frame positions using the encoder's frame-length convention and compute
  length from the difference. Pre-skip is outside these positions.
  [Ogg Opus specification](https://www.rfc-editor.org/rfc/rfc7845.html)
- MP3 can store the same names in ID3 user-defined text frames. Warn on every loop preset
  with MP3 because encoder delay and padding can cause a gap in players that do not
  compensate for them. Preserve the existing LAME header and verify that the decoder
  accepts the tagged output. [ID3 text frames](https://id3.org/id3v2.4.0-frames)

Report a Godot `loop_offset` hint in seconds from the start of the loop-bearing file.
Document how to enable looping and enter the offset for Ogg Vorbis and MP3. For WAV,
document detection from the embedded markers. A loop spanning to EOF also fits Godot's
Ogg Vorbis and MP3 import settings. [Godot audio import](https://docs.godotengine.org/en/4.5/tutorials/assets_pipeline/importing_audio_samples.html)

### Files and CLI output

Use descriptive preset suffixes for default single-file exports in `renders/`. For
`intro-loop` use `<stem>-intro.<ext>` and `<stem>-loop.<ext>`, treating an explicit `--out`
as the stem and format of that pair. Resolve both destinations through the existing
project checks, encode both before writing either, and use the current atomic writer for
each file. If the second write fails, report which path completed and which failed.
The same command can retry both writes. Preserve the existing path and output shape when
the flag is omitted. For presets report each written path, decoded frame range and sample
rate for loop files, and the Godot hint in human and JSON output. Retain the existing
usage and render failure exit classes.

## Dependencies

Task 14 supplies the host-backed renderer, encoders and project output handling and is
complete. Use the existing encoders and deterministic output paths. Changes to
`job-plan.ts` require rebuilding the committed host bundle and regenerating engine
reference provenance through the documented commands. Update the render guide, contributor
command reference and embedded CLI skill with the final flags and output.

## Risks

- A pattern near a bar boundary can still be releasing when its schedule ends. Test
  structured and plain patterns that would play different audio after the cutoff, and
  verify that only already-triggered voices contribute to post-roll.
- Tail decay estimates are conservative and very long feedback can make the buffer exceed
  existing limits. Test dry and wet sources, long tails, and sizes close to the render limit.
- Fractional frames per bar and the Opus resampling ratio can introduce one-frame errors.
  Test a fractional tempo and read loop markers and decoded lengths using independent
  readers, including WAV's inclusive end and Opus's pre-skip.
- Verify the folded result against a PCM reference and measure its wrap discontinuity on a
  wet fixture. Test an echo arriving after a silent gap and a multichunk export with one
  worker and several workers. Check the decoded intro-to-loop transition and wrap with
  leveling on and off.
- Test the CLI's default and preset paths, JSON, invalid flag combinations, missing loop
  sections, zero intro, MP3 warning, and write failures. Run host parity and the applicable
  render corpus alongside the normal Bun and Rust gates to protect regular renders.

## Open Questions

- Is the proposed CLI vocabulary right for the four presets?
- Should `one-shot` and `folded-loop` expose a tail duration override in the first release,
  or use the schedule-derived duration for both?
- Should the split preset keep the first pass as heard after the intro, as proposed, or
  make the separate body tail-folded by default?
