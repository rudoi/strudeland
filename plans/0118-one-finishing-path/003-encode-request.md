# 003: an explicit encode request

## Goal

One encode entry point takes a request that names the loop range and the MP3 flush. `Real` is
a unit struct: the flush no longer hides in a codec variant. The Opus loop-point resampling sits
beside the encoders. There is no behaviour change; the loop path still calls the encoder outside
the `Codec` seam until stage 005.

## Prerequisites

- **Stage 002 is committed,** and the characterization passes.
- **Read the proposal's** "Encode request (stage 003)" and Decisions 1 and 7.

## Implementation

1. **`src/encode/mp3.rs`:**
   - Add `pub enum Mp3Flush { NoGap, Gap }`, with derives `Debug, Clone, Copy, PartialEq, Eq`.
     Its doc names LAME's `FlushNoGap` and `FlushGap`, says what each means for a decoder, and
     says which callers use which.
   - Replace `encode_mp3` and `encode_mp3_with_loop` with one `pub fn encode_mp3(channels,
     sample_rate, mode, quality, flush: Mp3Flush, loop_range: Option<Range<u64>>)`. It matches
     the flush onto `encode_mp3_file::<FlushNoGap>` or `encode_mp3_file::<FlushGap>`.
   - Keep `encode_mp3_file` as it is.
2. **`src/encode/mod.rs`:**
   - `pub use mp3::Mp3Flush;`
   - `pub struct EncodeRequest<'a> { channels, sample_rate, encoder, mp3_flush, loop_range }`,
     as in the proposal, with field docs.
   - `pub fn encode_once(request: &EncodeRequest<'_>)`: the `match *request.encoder` of today's
     `encode_once_with_loop`, with MP3 going to `mp3::encode_mp3(.., request.mp3_flush,
     request.loop_range.clone())`.
   - Delete `encode_once_with_loop`.
   - `pub(crate) fn loop_points(format: Format, range: &Range<usize>, sample_rate: u32) ->
     (Range<u64>, LoopPoints)`: lift `loop_export.rs:361-378` verbatim. Opus uses
     `opus::resampled_len` on each end and rate `OPUS_RATE`; the other formats use the identity
     and `sample_rate`. `LoopPoints` comes from `crate::LoopPoints`.
   - **`Codec`:** make it `pub(crate) trait Codec`, with `fn encode(&self, request:
     &EncodeRequest<'_>)`. `decoded_true_peak_db` is unchanged.
   - **`Real`:** make it `pub(crate) struct Real;`, whose `encode` is `encode_once(request)`.
   - **`encode_with(codec, mp3_flush: Mp3Flush, input, ..)`:** build the request with `loop_range:
     None` and the given flush.
     - `encode` passes `&Real, Mp3Flush::NoGap`.
     - `encode_standalone` passes `&Real, Mp3Flush::Gap`.
3. **`src/loop_export.rs`:**
   - Replace the inline range and points block (`:361-380` at the plan commit) with
     `part.loop_range.as_ref().map(|r| loop_points(format, r, sample_rate))`, unzipped into the
     range and the points.
   - Replace `encode_once_with_loop(channels, sample_rate, &options.encoder, range)` with
     `encode_once(&EncodeRequest { channels, sample_rate, encoder: &options.encoder, mp3_flush:
     Mp3Flush::Gap, loop_range: range })`.
   - `verify::decoded_true_peak_db` stays direct here, until stage 005.
4. **Callers:**
   - `tests/encode.rs`: each `encode_once(x, SR, &enc)` becomes `encode_once(&EncodeRequest {
     channels: x, sample_rate: SR, encoder: &enc, mp3_flush: Mp3Flush::NoGap, loop_range: None
     })`. Add a local `fn once(x, enc)` helper; `:273-274` becomes `encode_mp3(.., Mp3Flush::Gap,
     ..)`.
   - `tests/pipeline.rs:9`/`:551`: the same.
   - Grep for any other `encode_mp3(`.

## Tests

- **Moved:**
  - `encode_mp3_preset_preserves_the_gapless_duration_and_loop_tags` now calls `encode_mp3(..,
    Gap, Some(..))` and `encode_mp3(.., Gap, None)`, and keeps every assertion.
  - `encode/mod.rs`'s stub implements the request signature. Its tests are unchanged.
- **New (`tests/encode.rs`):**
  - **`encode_mp3_flush_is_the_only_difference_between_the_finalizers`:** for the same input
    and mode, `NoGap` and `Gap` bytes differ. `Gap` with a range equals the ID3 tag plus `Gap`
    without one.
  - **`encode_once_honours_the_flush_only_for_mp3`:** for WAV, Ogg and Opus, `NoGap` and `Gap`
    requests give identical bytes.
- **New (`encode/mod.rs`): `loop_points_resample_each_end_for_opus`:**
  - Opus at 44.1 kHz: `147..294` gives `160..320`, and `LoopPoints { start_frame: 160,
    length_frames: 160, sample_rate: 48_000 }`.
  - `0..1` gives `0..2`.
  - WAV, Ogg and MP3 are the identity at 44.1 kHz.

## Validation

- **Targeted:** `cargo test -p strudeland-render --test encode` and `cargo test -p
  strudeland-render encode::`.
- **The stage gates,** logged once. `characterization` passes, and the golden is untouched.
- **`rg -n "encode_once_with_loop|encode_mp3_with_loop|Legacy|Standalone" crates/`** finds
  nothing.

## Commit message

```
refactor(render): name the loop range and MP3 flush in one encode request

Encoding takes an EncodeRequest that carries the loop range in decoded
frames and how an MP3 stream is finalized, instead of choosing the
flush through a codec variant. The real codec is a unit struct, and the
Opus loop-point resampling sits beside the encoders. Plain renders keep
the no-gap flush; one-shots and loop files keep the gap flush.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
```
