# strudeland-engine

A deterministic, single-threaded Rust renderer for Strudel haps that aims to sound as superdough
does in Chromium's offline audio context. The render harness calls the binary; later the app calls
the library. The plans are `plans/0010-rust-engine-core/` (the core voices) and
`plans/0011-engine-effects/` (worklets, effects, modulation, buses and FX chains).

```
cargo run -p strudeland-engine -- render --job <file|-> --out <file.wav>
cargo run -p strudeland-engine -- signatures --job <file|->
cargo run -p strudeland-engine -- controls --json
```

`render` reads an engine job (schema 2, written by the render page, `src/lib/render/engine-job.ts`;
schema 1 jobs are rejected, not migrated: regenerate them), writes a 32-bit float WAV
(`WAVE_FORMAT_EXTENSIBLE` above two channels) and prints a one-line JSON summary `{ ok, frames,
channels, errors, renderMs }`. Exit codes: 0 rendered without errors, 1 rendered with render
errors, 2 bad input or I/O. `signatures` prints what the engine cannot play in a job, one line of
JSON `[{ s, keys, firstBar }]` (`[]` when it plays everything; exit 0 for any valid job, 2 for bad
input). It is the only unsupported detection: `scripts/render/rust-engine.ts` calls it, and the
shared signature table `tests/data/unsupported-signatures.json` pins its rules. `controls --json`
prints the control allowlist (`capability::controls_json`; nothing on the TypeScript side mirrors
it). Its `sounds` are the sources the engine builds plus the silence markers, and its
`modulatorTargets` are the `CONTROL_TARGETS` rows that resolve on some voice
(`modulate::modulatable_targets`, pinned by a test).

## Chromium reference

Ported from Blink at Chromium tag **156.0.8078.4** (the build Playwright 1.64.1 drives), sources
under `third_party/blink/renderer/`. Ported files carry the header

```rust
// Port of Blink (Chromium 156.0.8078.4) <path>[, <path> ...].
// Copyright 2012 The Chromium Authors. BSD-3-Clause; see LICENSE-chromium.
```

and `tests/licence.rs` enforces it. Ported:

| Module | Blink sources |
|---|---|
| `render` | `modules/webaudio/offline_audio_context.cc` (suspend quantization), `platform/audio/audio_utilities.cc` (`TimeToSampleFrame`, `RoundUpToMultiple`) |
| `param` | `modules/webaudio/audio_param_handler.cc` (the timeline: event insertion and pruning, `ValuesForFrameRangeImpl`, linear and exponential ramps, `HasSampleAccurateValues`, k-rate and a-rate values; the cancel events of `cancelAndHoldAtTime` for duck: `CancelAndHoldAtTime`, `RemoveCancelledEvents`, `HandleCancelValues`, `ProcessCancelValues`; SetTarget, SetValueCurve and `cancelScheduledValues` are not ported), `modules/webaudio/audio_param.cc` (the `.value` setter) |
| `fft` | `platform/audio/rustfft_ffi.rs` (the even-size real FFT on a half-size complex `rustfft` transform and its packing), `platform/audio/fft_frame.cc` |
| `periodic` | `modules/webaudio/periodic_wave_handler.cc` (`CreateBandLimitedTables`, `GenerateBasicWaveform`, `NumberOfPartialsForRange`, the scalar and NEON `WaveDataForFundamentalFrequency`), `modules/webaudio/periodic_wave.cc` (`createPeriodicWave` checks) |
| `osc` | `modules/webaudio/oscillator_handler.cc` (`Process`, the a-rate and k-rate paths, `DoInterpolation`), `modules/webaudio/cpu/arm/oscillator_kernel_neon.cc` (`ProcessKRateVector`, `ProcessARateVectorKernel`), `modules/webaudio/audio_scheduled_source_handler.cc` (`UpdateSchedulingInfo`, `start`/`stop`), `modules/webaudio/constant_source_handler.cc` |
| `biquad` | `platform/audio/biquad.cc` (`SetLowpassParams`, `SetHighpassParams`, `SetBandpassParams`, `SetNotchParams` for the phaser, `SetNormalizedCoefficients`, `Process`, the macOS `ProcessFast`, `TailFrame`), `modules/webaudio/biquad_filter_handler.cc` (`Process`, `ProcessOnlyAudioParams`, `NormalizeFrequency`, `HasConstantValues`, the tail time), `modules/webaudio/biquad_filter_node.cc` (param ranges), `modules/webaudio/audio_handler.cc` (`ProcessIfNecessary`, `PropagatesSilence`) |
| `panner` | `platform/audio/stereo_panner.cc` (`PanWithSampleAccurateValues`, `PanToTargetValue`), `modules/webaudio/stereo_panner_handler.cc`, `modules/webaudio/stereo_panner_node.cc` |
| `noise` | `modules/webaudio/audio_buffer_source_handler.cc` (`Process`, `RenderFromBuffer` at rate 1 with loop on: the start-time skip, `ProcessFastPath`, `ProcessInterpolatedPath`) |
| `orbit` | `modules/webaudio/audio_node_input.cc` (`SumAllConnections`), `modules/webaudio/audio_summing_junction.cc`, `platform/audio/audio_bus.cc` (`SumFrom`, `SumFromByUpMixing`), `platform/audio/audio_channel.cc` (`SumFrom`: copy into silence, add otherwise), `modules/webaudio/channel_splitter_handler.cc`, `modules/webaudio/channel_merger_handler.cc` |
| `delay` | `platform/audio/delay.cc` (`ProcessARate`, `CopyToCircularBuffer`, the buffer length), `platform/audio/cpu/arm/delay_neon.cc` (`ProcessARateVector`: the float kernel), `modules/webaudio/delay_handler.cc` (`Process`, `ProcessOnlyAudioParams`, `CheckNumberOfChannelsForInput`: the up-mixed history, the tail time), `modules/webaudio/delay_node.cc` (`delayTime` range), `modules/webaudio/audio_handler.cc` (`ProcessIfNecessary`: the feedback read one quantum late, silence), `modules/webaudio/audio_node_output.cc` (`UpdateInternalBus`: the output bus re-created on a channel change) |
| `reverb` | `platform/audio/reverb.cc` (`CalculateNormalizationScale`, `Process`: mono input to both channels), `platform/audio/reverb_convolver.cc` (the stage layout), `platform/audio/reverb_convolver_stage.cc` (pre- and post-delays, the render phases), `platform/audio/reverb_accumulation_buffer.cc`, `platform/audio/direct_convolver.cc`, `platform/audio/fft_convolver.cc` (overlap-add), `platform/audio/fft_frame.cc` (`Multiply`, `DoPaddedFFT`, `ScaleFFT`), `modules/webaudio/convolver_handler.cc` (`SetBuffer`, `Process`, the tail time), `modules/webaudio/audio_handler.cc` (silence after the tail), `modules/webaudio/audio_buffer_source_handler.cc` and `modules/webaudio/offline_audio_context.cc` (the nested context that renders superdough's IR lowpass: the buffer copied from frame 0, 128-frame quanta, the first `N` frames kept) |
| `compressor` | `platform/audio/dynamics_compressor.cc` (the kernel: static curve, makeup gain, envelope rates, detector, look-ahead), `modules/webaudio/dynamics_compressor_handler.cc` (`Process`, latency and tail time), `modules/webaudio/dynamics_compressor_node.cc` (param ranges), `platform/audio/audio_utilities.cc` (`DecibelsToLinear`, `LinearToDecibels`) |
| `worklet` | `modules/webaudio/audio_worklet_handler.cc` (`Process`: params rendered every quantum, the active-source and connected-input rule, a throw finishes the processor), `modules/webaudio/audio_worklet_processor.cc`, `modules/webaudio/audio_worklet_global_scope.cc` (`currentTime`), `modules/webaudio/base_audio_context.cc` |
| `voice` | `modules/webaudio/gain_handler.cc` (`Process`, `ProcessOnlyAudioParams`), `modules/webaudio/audio_handler.cc` (silence propagation, `ProcessIfNecessary`'s once-per-quantum rule for the FM graph), `platform/audio/audio_bus.cc` (`CopyWithGainFrom`, `CopyWithSampleAccurateGainValuesFrom`, `SumFrom`) |

The other ports follow superdough's JavaScript and carry no Blink header. They include the worklet
processors (`supersaw`, `shaper`, `transient`, `ladder`, `djf`, `lfo`, `envelope`, from
`worklets.mjs`), `vowel` (`vowel.mjs`), `modulate` (`modulators.mjs` with the two vendored
patches), `bus`, `chain` (superdough's FX loop and node tracker), `adsr`, and `js`.
The browser and Rust pulse implementations share Strudeland's
[pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md).
`jsmath` ports V8 and CORE-MATH (see V8 math).

Chromium builds its FFT (`FFTFrame`) on the `rustfft` crate at this tag. The engine pins the same
versions (`rustfft` 6.4.1 with its default features, so the NEON planner on arm64, and
`num-complex` 0.4.6) and ports Blink's packing, so the periodic-wave tables and the convolver's
FFTs are bit-identical to Chromium's; bump them together with the tag. (`realfft` is not used:
its packing and scaling differ from Blink's.)

Every voice feeds its orbit (`orbit.rs`, superdough's `Orbit` and `SuperdoughOutput`): the orbits
of `job.orbits` are created up front in that order, each a stereo summing node (mono voices
up-mixed to both channels), a gain of 1, a pan-0 `StereoPannerNode` (its stereo law adds the
right channel times `cos(pi/2)`, about 6.1e-17, to the left) and a splitter whose two outputs go
to the merger channels `channels[i] % count` (the browser harness aliases stems past 16 pairs; a mono job gets both
sides on its one channel). A voice connects `post` (the last gain) or, with `dry`, `Gain(dry)` of
it; its delay send feeds the orbit's feedback delay and its reverb send creates or updates the
orbit's reverb and feeds its convolver. A hap finds its orbit by `orbit_number`; superdough keys
orbits by `String(orbit)`, so two values with one number (`1` and `'x'`) share the first one's bus
in the engine. Blink keeps a summing junction's connections in a hash set, so the order it adds
voices (and orbits sharing a channel) in is not defined; the engine adds them in connection order,
so sums of three or more voices can differ from Chromium's in the last bit (Chromium's own order
also changes between runs).

Offline stem export builds one `StemLayout` across the scheduled haps of all checked stereo mix
jobs, including lookback. Each `render_with_stems` call returns the ordinary stereo mix followed
by a stereo pair for every layout entry. Orbit stems capture the routed orbit outputs by numeric
identity. Voice and instrument stems sum the connected direct signals after the orbit's duck gain
and fixed panner. The orbit's shared-effects pair carries the difference from its contribution to
the mix, including delay, reverb, orbit filtering, and merger rounding. A bus reader contributes
the audio it reads to its own stem.

The orbit's feedback delay (`delay.rs`, superdough's `FeedbackDelayNode` and `Orbit.getDelay`) is
created by the first hap with a delay send (`delay > 0 && delaytime > 0 && delayfeedback > 0`):
`delayTime.value = delaytime` (clamped to the 1 s maximum), feedback `min(clamp(fb, 0, 0.98),
0.995)`, an output gain of 1 (superdough's `start(t)` re-schedules that 1 and is omitted). Every
send then runs `getDelay`'s guard: `setValueAtTime` at the hap's `t` only when the param's float
`.value` (the last rendered or set value, not the last scheduled one) differs from the JS double,
so values not exact in float and anything above 1 s always schedule, and within one pause a
return to an exact value is skipped. The line is Blink's a-rate delay (written before it is read,
linear interpolation in float as the NEON kernel computes it, 44228 frames); the feedback gain
reads the delay's output of the previous quantum, so each loop adds 128 frames (echo k at `t0 +
k*D + (k-1)*128`). It is mono until a stereo (panned) send connects, then stereo for good: the
history is up-mixed with the same write index, and the feedback reads silence for that one
quantum (Blink re-creates the node's output bus). Its return enters the summing node before the
voices; the sends (taken from `post`, before `dry`) are summed after the feedback in voice order.
The orbit's reverb (`reverb.rs`, superdough's `createReverb` and `Orbit.getReverb`, and
`reverbGen.mjs`) is created by the first hap with `room > 0` with its `roomsize`, `roomfade`,
`roomlp`, `roomdim` (defaults 2 s, 0.1 s, 15000 Hz, 1000 Hz for undefined ones). A later hap asks
for a new IR when one of its defined settings differs strictly from the stored one; the request
uses its own settings with the defaults for the rest, and every request stores its settings and
takes a request number even when it fails. The IR (`N = round(1.5 * roomsize * 44100)` frames of
seeded noise decaying to -60 dB at `roomsize`, faded in over `roomfade`) is generated when the
pause is settled, for the newest request of the pause that did not fail, and replaces the
previous one; it is bit-identical to `generateReverb`'s (`tests/vectors/reverb-ir.json`). Unless
`roomlp` is falsy (or loosely 0) it is rendered through Blink's biquad lowpass as superdough's
nested `OfflineAudioContext` does (`roomlp` ramping linearly to `roomdim` at `roomsize`, Q 0.0001
dB), bit-identical to Chromium's on the reference machine. `N < 1` (a `roomsize` of 0, negative,
NaN, ...) is an `ir` render error at the hap's bar and a lowpass frequency that is not a finite
float (`-Infinity`, a non-numeric string) a `hap` error; either way superdough throws out of the
hap's build: the previous IR stays, and the voice keeps only the sends it connected before
(a delay send made earlier keeps sounding) but never reaches the summing node. The same holds for
any throw after a send (a `room` or `dry` level that is not a finite float).
Each IR that lands builds a new convolver (`ConvolverNode.buffer =`), so a swap at a pause frame
drops the old tail and the input history. The convolver is Blink's `ReverbConvolver` per IR
channel (render phases 0 and 128): a direct stage over the first 64 taps, FFT stages of 64 taps at
FFT 128 doubling to 4096 taps at FFT 8192, then 4096-tap stages to the IR end, each with Blink's
pre- and post-delays into an accumulation buffer, so the output has no latency and the FFT blocks
fall where Chromium's do. The normalization scale multiplies the direct kernel and the FFT
kernels' spectra (as Blink), the spectra are multiplied in Accelerate's `vDSP_zvmul` order
(measured, checked by a macOS test), and the FFTs are Chromium's own. The room sends of an orbit
(each `post` times its level, in voice order) are summed into the convolver's input, mono while
every send is mono (both IR channels then convolve it); the return joins the summing node before
the voices, after or before the delay return in creation order. Once its input has been silent for
the IR's length the node outputs silence without running (Blink's tail rule), so its state waits.
Two rounding-level deviations keep the wet path from being bit-exact (about 1e-6 relative, well
below -100 dB): the normalization's sum of squares (`vDSP_svesq`, see below) and the direct
stage's sum (Accelerate's `vDSP_conv`, whose order depends on the buffer's alignment; the engine
sums in ascending lag order with fused multiply-adds).
The oscillator sounds (built-in and `waveformN` periodic waves, vibrato, pitch envelope, FM,
the `noise` mix), the noise sounds and `one` are final, as is the rest of the voice: the amp
envelope (an a-rate `param::Param` scheduled by `adsr::get_param_adsr` and applied as
`GainHandler::Process` does), the gain stage, the 12 and 24 dB biquad filters with their envelopes,
the stereo panner, `postgain` and `dry`.

The biquad's coefficients are Blink's RBJ forms (Q in dB for lowpass and highpass through
`fdlibm::expf`, so the resonance has float precision; linear Q for bandpass; `fdlibm` `sin`/`cos`).
Its per-frame kernel is the double direct form with float-rounded output history; its constant
kernel is the macOS one, Accelerate's `vDSP_deq22D`, whose evaluation order was measured on the
reference machine (`fma(-a2, y2, fma(-a1, y1, fma(b0, x, fma(b2, x2, b1 * x1))))`, double history;
a macOS-only test checks the port against Accelerate). The panner's stereo k-rate branch uses
`vDSP_vsma`, a fused multiply-add (also checked against Accelerate). Each node in the voice chain
carries Blink's silence hint, so a filter rings on after its input goes silent until its tail time
(`TailFrame`, 1/32768 of the impulse response) has passed and then outputs silence, as Chromium's
does; the voice is freed when its sources have stopped and its output stayed below 1e-10 for a
quantum. Chromium also disconnects a voice when superdough's main-thread `onended` cleanup runs,
which races the offline render (a long ring is cut at a quantum that varies from run to run, and
the destination's summing order changes, so sums of three or more voices differ in the last bit
between runs); the engine does not model it.

Noise buffers (`noise.rs`) are generated as `noise.mjs` does, bit for bit (float32 stores, the
brown recurrence and pink's second rounding read back); white, pink and brown are cached for the
run from the job's seed, crackle is rebuilt per voice from its voice seed. They play through a
port of Blink's `AudioBufferSourceNode`: the source starts on `TimeToSampleFrame(t, RoundUp)`, and
when `t` lies before that frame Blink skips the fraction (`virtual_read_index += |offset|`), which
leaves the read index fractional for good, so the source then interpolates linearly between
samples (in double, looping through the first sample); an on-frame start (or one that the 1/1024
rounding moved down) copies samples. FM (`applyFM`) builds the modulators in matrix order; each
starts with `start()` at the scheduling frame but, as every Blink node, renders only when pulled,
so it first plays in the quantum the carrier first evaluates its `frequency` (its first playing
quantum). Each node renders once per quantum; a cycle in the matrix (`fmi11`, or `fmi2` with
`fmi12`) reads the re-entered oscillator's previous output, as Blink's `ProcessIfNecessary` guard
does. A param sums its connections one after the other into its values; Blink keeps them in a hash
set, so with two or more connections into one param its order, and the last bit of the sum, may
differ from the engine's matrix order.

Deliberate deviations: a custom wave whose coefficients overflow the float range once scaled
(Chromium would render NaN) is refused with a hap error; a voice whose output becomes NaN or
infinite (a lowpass or highpass Q below about -758 dB underflows the resonance to 0, gains whose
product overflows the float range) is silenced from that quantum on and reported as a hap error,
and a mix sample that overflows is written as 0 and reported once (`other`); Chromium writes the
NaN or infinite samples. An IR request generates at most `2^26` frames (a longer
`roomsize` is the `ir` error, as `N < 1` is), and a fade longer than the IR fades the IR only
(JS keeps looping past its end, forever for an infinite `roomfade`). The IR normalization sums
squares in double where Blink calls Accelerate's `vDSP_svesq` (float, an order that could not be
reproduced; both are within about 1e-6 of the exact sum). The convolver's direct stage sums its
64 taps in ascending lag order with fused multiply-adds where Blink calls `vDSP_conv` (an order
that depends on the buffer's alignment; the same sum to float rounding, checked against
Accelerate by a macOS test).

## Effects, worklets, modulation and routing

A voice is its source and a chain of stages (`chain.rs`) processed in place on a one- or two-channel
bus, built per FX entry in superdough's order (superdough.mjs:612-1004): the `transient` worklet,
the gain stage, the lowpass, highpass and bandpass filters (biquads, two each for 24 dB, or the
`ladder` worklet for `ftype('ladder')`) with their filter LFOs, the vowel, the `coarse`, `crush`,
`shape` and `distort` worklets, the tremolo, the compressor, the panner, the phaser; after the last
entry the `FXrelease` gain, `post` (`postgain`) and `dry`. `NodeTable::plan` decides the stages
once: the table holds the stage specs `Voice::build` walks (each built by `build_stage`, in
superdough's throw order) and superdough's `nodes` tracker, which the modulators resolve their
targets on. The compressor and the panner output two channels; a stereo source (`supersaw`,
`pulse`, the `bus` sound) keeps the voice stereo.

- **Worklets.** `worklet.rs` is how Chromium runs an `AudioWorkletProcessor` offline: one
  full-range param per descriptor unless it spells `minValue`/`maxValue`, the params rendered
  every quantum (so a worklet source's vibrato and FM modulators are pulled from the voice's
  scheduling frame), `currentTime` the quantum's start (a worklet source first runs in the first
  quantum with `currentTime > f32(begin)`, `evidence/measure-worklet-start.json`), a processor
  called while it is an active source or has a connected input, and the teardown cascade (a
  worklet after a stopped source loses its input in the quantum after the stop,
  `evidence/measure-worklet-teardown.json`). The processors follow `worklets.mjs` on
  `jsmath`: `supersaw`, `shaper` (`coarse`, `crush`, `shape`, the nine `distort` algorithms),
  `transient`, `ladder`, `djf`, `lfo` and `envelope`. Both engines render the pulse as two
  identical channels with four-point edge correction and a two-frame oscillator delay.
- **Native effects.** The compressor is Blink's `DynamicsCompressorNode` (a fresh node per hap; a
  node created before frame 71936 of a chunk warms up on silence through the quantum at 71808,
  `evidence/compressor-warmup.json`), its `powf`/`log10f`/`exp`/`sin` the platform's (Apple's libm
  on the reference machine, `evidence/compressor-libm.json`). The vowel is five bandpass biquads
  into `Gain(8)` (`vowel.rs`), the phaser a notch biquad whose `detune` an `lfo` worklet drives
  (float `exp2f` in the detune path, `evidence/phaser-exp2.json`), the tremolo an AM gain with an
  `lfo` on its gain, `FXrelease` a gain holding 1 until `end + release` and fading linearly to 0.
  The filter LFOs (`lprate`, `hprate`, `bprate` and their keys) and the pulse's width modulation
  (`pwrate`, `pwsweep`) are `lfo` worklets connected to a param.
- **Orbit effects.** `djf` is the orbit's DJ filter worklet, inserted between the summing node and
  the output by the first hap with `djf != null` and kept for good. Duck ports `offlineDuck`
  (`src/lib/audio/offline.ts`, the curve model the references render): per target orbit a
  `cancelAndHoldAtTime` and two exponential ramps on the orbit output's gain, the held value from
  the JS-side curve model (`evidence/duck-first-ramp.json`). A change to `offlineDuck` moves the
  engine, the goldens and the refs together.
- **Modulators.** `modulate.rs` ports `modulators.mjs` with the `modulator-fxi-copy` and
  `bmod-gate` patches: `lfo` and `env` build an `lfo` or `envelope` worklet, `bmod` reads a bus
  through a DC offset, a depth gain and a gate open from `begin` to `endWithRelease`, each
  connected to the params its control resolves to (`getControlData`, the `fxi` entry, `currentValue`
  from the param's `.value`, `evidence/mod-lfo-gain.json`, `evidence/mod-orbit-delaytime.json`).
  Superdough's logged skips (no control data, no node under the key) connect nothing and are
  reported with superdough's log text as `other`; its throws are `hap` errors.
- **Buses.** `bus.rs`: one two-channel `GainNode(1)` per `String(bus)`, fed by the voices' `bus`
  sends (`busgain`) and read by the `bus` sound and `bmod`. The job's bus graph is checked at the
  value level; a cycle is refused (`bus-cycle`), an acyclic graph renders level by level.
- **FX chains.** Every entry of `FX = [...value.FX, value]` gets its own stages; a chain entry ends
  with its local feedback delay and local reverb (a dry and a wet gain summed into a stereo sum).
  Their tails render the quantum holding `endWithRelease` and are cut from the next one.

## V8 math

superdough's worklet processors compute in JavaScript, so the engine needs V8's `Math.*` bit for
bit. `jsmath` provides it: `sin`, `cos`, `exp`, `expm1`, `log1p` are correctly rounded ports of
CORE-MATH (V8's values are correctly rounded on every reference point), `cosh` a port of V8's own
code on those, and `pow` and `tanh` the host C library's (declared against the C symbols, not
Rust's `std`), as V8 calls them (`pow` behind V8's `math::pow` special cases, `use_std_math_pow`).
Since Chromium's `pow` and `tanh` are host-dependent too, they match Chromium on the same host:
`tests/v8_math.rs` judges them (and the `distort:*` groups, which call `tanh`) on macOS only and
records their rates elsewhere. `round` and `toInt32` are JS semantics. `jsmath::IMPLEMENTATION`
names the choice per function; Rust `std` is never used.

The reference is `tests/refs/v8-math.json`: the grid of `scripts/engine/v8-math-cases.ts` (inputs
derived from the processors, plus the nine `distort:<name>` curve groups) evaluated inside an
AudioWorklet by the render page's `math` page kind (`src/render/math.ts`, `math-worklet.ts`), twice
in fresh pages, by `bun run engine:refs`; its provenance hashes the grid's inputs (`inputsSha256`).
`tests/v8_math.rs` requires the chosen implementation to match every vector (100 %) and records the
`libm` crate's and `std`'s rates for the record
(`plans/0011-engine-effects/evidence/v8-math-choice.json`: on the transcendentals `libm` matches 91
to 98 % of the vectors, `std` 84 to 100 %). Ported files carry their family's header (`// Port of
CORE-MATH (` with `LICENSE-core-math`, `// Port of V8 (` with `LICENSE-v8`); `tests/licence.rs`
checks that each licence file exists exactly when a file carries its header.

## JS vectors

`src/js.rs` ports superdough's seeding (`rng.mjs`: `hashSeed`, `mulberry32`, `makeRandom`,
`stable`, `voiceSeed`) and JS `String(number)` bit for bit. `tests/vectors/js-*.json` pin them; they
are written only by `bun run engine:vectors --write` (`scripts/engine/vectors.ts`, Bun only, from
the vendored superdough modules; without `--write` it prints a diff summary and exits 1 when they
differ), and `bun test` fails when they are stale. Regenerate after a Strudel sync. The voice seed
hashes the value after superdough's mutation (`duration`, `bank` prefix); silence markers do not
advance the occurrence counter. Jobs are parsed with serde_json's `float_roundtrip`, so the hashed
numbers have the bits JS wrote.
`adsr.json` pins `getADSRValues` and the event list `getParamADSR` schedules (exact),
`frequency.json` `getFrequencyFromValue` (within 1 ulp: Bun's JavaScriptCore `Math.pow` may differ
from V8's in the last bit), `waveform-n.json` superdough's `waveformN` `real`/`imag` arrays (float
values within 1 ulp; errors by class; generated with a stub context whose `createPeriodicWave`
applies Blink's checks).
`reverb-ir.json` pins `generateReverb` without the lowpass (05's grid of `roomsize` 0.05, 0.5, 2 by
fade 0, 0.1 by two seeds, plus one- and two-frame IRs, a fade longer than the IR and non-numeric
settings): every short IR whole, the first and last 1024 samples of the others, and a hash of each
channel; samples within 1 ulp, the hashes currently equal.
`noise-{white,pink,brown}.json` pin `getNoiseBuffer` for two audio seeds and `noise-crackle.json` for
two voice seeds at densities 0.02 and 2 (exact: the first and last 1024 samples as f32 bits and an
FNV-1a hash over the little-endian bytes of all 88200).

## References

`tests/refs/` holds the Chromium references, written only by `bun run engine:refs --write
[name...]` (`scripts/engine/refs.ts`; without `--write` it prints a diff summary and writes
nothing):

- micro cases (`tests/render/engine-micro/<name>.js`, one DSP rule each): `<name>.job.json` (the
  job, with `provenance`) and `<name>.wav` (superdough's render in Chromium). `tests/micro.rs`
  renders each job. On the reference Chromium OS and architecture it requires the null residual `20*log10(rms(chromium - engine) /
  rms(chromium))` to stay below the case's `maxResidualDb` in `tests/refs/thresholds.json`, which
  cites its evidence under `plans/0010-rust-engine-core/evidence/` or
  `plans/0011-engine-effects/evidence/`. A case with `platform: "macos"` (the four `compressor-*`
  cases, whose kernel calls the platform's libm) is null-tested on macOS. On other platforms,
  the micro tests check shape, silent channels, finite samples, error contracts and exact
  same-host repeatability. The fixture-envelope tests still run across platforms. A measured-floor case
  (`floor` in `thresholds.json`, one of `FLOOR_CASES` in `scripts/engine/refs-lib.ts`:
  `vowel-formants`, `mod-lfo-note`) has a junction of three or more sounding inputs that Chromium
  sums in hash-set order, so its reference varies in the last bit between runs: `engine:refs`
  renders it eight times and records the largest residual of renders 2 to 8 against render 1 as
  `floorDb` in `index.json` (a deterministic case is an error), and a dry run counts it current
  while every render is within `floorDb + 6 dB` of the committed WAV. Floor mode is only for
  those named junctions, never to absorb a defect;
- the V8 math reference `v8-math.json` (see V8 math);
- corpus fixtures with refs: `fixture-<name>--{mix,stems}-<k>.job.json` per chunk and the sibling
  superdough reference envelope `*.ref.json` (no WAV);
- `index.json`: every file's sha256 and size and every case's provenance; `bun test`
  (`tests/engine-artifacts.test.ts`) fails when a ref is stale, hand-edited or over budget.
  Provenance (`scripts/engine/provenance.ts`) records the case source (a fixture's song with its
  window, layout and gates), the vendored Strudel commit, `strudelPatchesSha256` (`PATCHES.md`
  and every file it lists), `plannerSha256` (`PLANNER_FILES`: `src/lib/render/plan.ts`,
  `engine-job.ts`, `lookback.ts` and `schedule.ts`; the lookback sets a fixture job's window),
  the Playwright pin, the Chromium version, and the platform; every field but `platform` must
  match. `v8-math.json` has its own provenance (`inputsSha256` of the grid, the Playwright pin and
  the Chromium version).

Regenerate after a Strudel sync or a local patch change (`PATCHES.md`), a job planner change, an
edit to `scripts/engine/v8-math-cases.ts`, a Playwright bump, an intended reference change, or a
change to a micro case or a fixture with refs. A dry `bun run engine:refs` must report "refs are
current": a micro case keeps at most two sounding inputs on any summing junction (Chromium's order
for three is not fixed; the floor cases are the named exceptions) and no voice whose ring
superdough's disconnect could cut inside the window (see Known differences; `FXrelease` fades such
a ring to exactly 0 first); a case that renders differently from run to run is redesigned, not
rewritten. Fixture envelopes are compared byte for byte, and superdough's fixture
renders are not bit-exact (voice-filter's 6-decimal channel peak flips in about 1 of 6 renders,
`plans/0011-engine-effects/evidence/load-variation.json`), so a dry run can report such a fixture
stale.

Fixtures are judged twice. `tests/fixtures.rs` renders every fixture chunk job and compares it
with the superdough reference envelope (10 ms window RMS and peak per channel) within the
`fixtures` tolerances of `thresholds.json` (window mean 0.05 dB, p95 0.1 dB, peak 0.05 dB, over
windows above -60 dBFS; `evidence/engine-fixtures-envelope.json`: the engine measures at the
envelope's own 0.01 dB rounding). `bun run render:check --engine rust` renders the whole corpus and
judges every supported fixture by all its gates and by parity against the superdough goldens with
`PROFILES.engine` (`src/lib/render/parity.ts`), calibrated by
`scripts/render/probes/engine-null.probe.ts` (`plans/0010-rust-engine-core/evidence/`
`engine-null-final.json` and `calibration.md`; re-checked with the effect fixtures, unchanged, in
`plans/0011-engine-effects/evidence/calibration.md`).

Evidence paths in the table are relative to the plan that added the case: from `osc-sine` to
`measure-suspend-quantization` `plans/0010-rust-engine-core/`, from `coarse-8` on
`plans/0011-engine-effects/` (the effect cases, all re-checked in its `evidence/calibration.md`).

| Micro case | Pins | Threshold |
|---|---|---|
| `osc-sine` | the built-in sine table at 55, 440 and 4186 Hz, default envelope | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-sawtooth` | the built-in sawtooth table at 55, 440 and 4186 Hz (k-rate NEON kernel) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-square` | the built-in square table at 55, 440 and 4186 Hz | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-triangle` | the built-in triangle table at 55, 440 and 4186 Hz | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-partials` | `waveformN` custom waves: `n` (count) and `partials` (magnitudes) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-phases` | `waveformN` phase rotation (`phases`) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-user` | `s('user')` with partials, and its triangle fallback | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-one` | `one` (ConstantSource): whole-frame start, no sub-sample phase | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-subsample-start` | fractional onsets on a k-rate first quantum: start phase `-offset * incr` | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-arate-start` | a-rate first quantum: phase 0 whatever the offset (Blink's local `frequency` quirk) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `osc-vibrato` | `vib`, `vibmod`: sine LFO through `Gain(vibmod * 100)` into `detune` | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `pitch-env-linear` | linear pitch envelope on `detune` (`penv`, `pattack`, `pdecay`, `psustain`, `prelease`) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `pitch-env-exp-negative` | exponential pitch envelope with negative `penv` (`pcurve(1)`) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `param-exp-ramp` | `exponentialRampToValueAtTime` (via `pcurve(1)`, `panchor(0)`) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `param-exp-held` | exponential ramp with opposite-sign ends, held by Blink | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `adsr-defaults` | the default amp envelope (`attack`, `decay`, `sustain`, `release` absent) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `adsr-attack-only` | attack only: sustain 1 (`getADSRValues`) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `adsr-sustain-only` | sustain only | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `adsr-decay-only` | decay only: sustain 0.001 | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `adsr-long-attack` | attack longer than the hap (`a > dur`), on `one` | -140 dB (`evidence/engine-null-micro-envelopes.json`) |
| `adsr-short-hap` | attack + decay longer than the hap (`a + d > dur`), fractional onsets, on `one` | -140 dB (`evidence/engine-null-micro-envelopes.json`) |
| `param-linear-ramp` | linear ramps between fractional frames (Blink's float `k` and `x`), on `one` | -140 dB (`evidence/engine-null-micro-envelopes.json`) |
| `filter-lp-const` | constant lowpass with resonance: RBJ coefficients, the `vDSP_deq22D` fast path | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `filter-lp-env` | lowpass envelopes (`lpenv` positive and negative, `lpattack` ..., `fanchor`): per-frame coefficients | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `filter-hp-const` | constant highpass with resonance (`hcutoff`, `hresonance`) | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `filter-hp-env` | highpass envelope (`hpenv`, `hpattack`, `hpdecay`, `hpsustain`) | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `filter-bp-const` | bandpass with linear Q (`bandf`, `bandq`, default Q 1) | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `filter-bp-env` | bandpass envelope (`bpenv` and its ADSR) | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `filter-24db` | `ftype('24db')` and `ftype(2)`: two cascaded biquads, each with its envelope | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `filter-ftype-numeric` | numeric `ftype` 1, 4 (12 dB biquad, not the ladder), 5 (24 dB), 0 | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `filter-tail-resonant` | resonant filters ringing past the source stop until Blink's tail rule silences them (voice lifetime) | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `pan-mono-center` | `pan` on a mono voice: the equal-power mono law, centre 0.7071 | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `pan-sweep` | one pan per hap across and beyond the range: a-rate then k-rate (float gains), clamping | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `pan-orbit-stems` | panned (stereo) voices through two orbits' summing nodes and pan-0 panners, with and without `dry` (pins the `cos(pi/2)` leak) | -300 dB (`evidence/engine-null-micro-orbit.json`) |
| `orbit-mix-two` | two orbits summed onto the mix pair at the merger, a mono and a panned voice | -140 dB (`evidence/engine-null-micro-orbit.json`) |
| `level-gain-velocity` | the gain stage `gain * velocity` (defaults, gain 0) | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `level-postgain-dry` | `postgain` and `dry` Gains, with and without pan | -140 dB (`evidence/engine-null-micro-filters.json`) |
| `noise-white` | white noise (seed 0): on-frame starts (fast path) and a fractional one (skip, interpolation) | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `noise-pink` | pink noise (`meta.seed` 7), one shared buffer | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `noise-brown` | brown noise (`meta.seed` -3), overlapping voices at quarter-frame offsets | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `noise-crackle` | crackle per voice seed at densities 0.02, 2, 30 and 0 | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `noise-mix` | the `noise` mix (`wetfade` dry/wet gains, the cached pink buffer) | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `noise-start-roundup` | buffer-source start frames: an onset rounded down by the 1/1024 rule (fast path) and half-frame onsets (skip) | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `fm-pair` | `fmi`/`fmh` pairs: modulator at the carrier's float frequency times `fmh`, the carrier a-rate | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `fm-chain-env` | chained modulators (`fmi2`, `fmi31`), linear and exponential FM envelopes, `fmwave3('square')` | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `fm-noise-wave` | noise modulators (white, crackle at density 2, brown) and a connection to a noise modulator skipped | -140 dB (`evidence/engine-null-micro-noise-fm.json`) |
| `measure-feedback-echo` | the feedback cycle: echo k at `t0 + k*D + (k-1)*128` with amplitude `g^(k-1)` (`evidence/measure-feedback-echo.json`) | -140 dB (`evidence/engine-null-micro-delay.json`) |
| `delay-impulse-echoes` | two orbit delays: an interpolated delay time (2756.25 frames) and the `delaysync / cps` default | -140 dB (`evidence/engine-null-micro-delay.json`) |
| `delay-guard-f32exact` | the guard with float-exact values: a return to the held value within one pause is skipped | -140 dB (`evidence/engine-null-micro-delay.json`) |
| `delay-guard-nonexact` | the guard with values not exact in float: always scheduled | -140 dB (`evidence/engine-null-micro-delay.json`) |
| `delay-time-change` | sample-accurate delay steps while the line holds signal, a delay below one quantum (write before read) | -140 dB (`evidence/engine-null-micro-delay.json`) |
| `delay-time-over-max` | `delaytime` above 1 s renders as 1 s; the guard against `.value` 1 | -140 dB (`evidence/engine-null-micro-delay.json`) |
| `delay-mono-to-stereo` | a panned send joins a ringing mono delay: history up-mixed, one quantum of feedback dropped | -140 dB (`evidence/engine-null-micro-delay.json`) |
| `reverb-ir-nolowpass` | a 0.25 s room without the IR lowpass: IR generation, normalization and every convolver stage kind (direct, doubling FFT stages, 4096-tap stages with pre-delays) | -140 dB (`evidence/engine-null-micro-reverb.json`) |
| `reverb-ir-lowpass` | the nested IR lowpass (`roomlp` ramping to `roomdim`) and a short `roomfade` | -137 dB (`evidence/engine-null-micro-reverb.json`) |
| `reverb-ir-swap` | a new IR every bar, each landing at its pause frame and cutting the old tail and input history | -142 dB (`evidence/engine-null-micro-reverb.json`) |
| `reverb-room-send` | room send levels, a dry-0 panned send making the convolver input stereo, sticky settings, a second orbit's own reverb | -148 dB (`evidence/engine-null-micro-reverb.json`) |
| `reverb-roomsize-zero` | `roomsize(0)`: the `ir` error at bar 1, the previous IR kept, the voice heard only through its earlier delay send | -149 dB (`evidence/engine-null-micro-reverb.json`) |
| `measure-suspend-quantization` | pause frames `round_up_128(trunc(suspendSec * sr))`; a hap sounds at its own start frame (`evidence/measure-suspend-quantization.json`) | -140 dB (`evidence/engine-null-micro-oscillators.json`) |
| `coarse-8` | `coarse(8)`: every 8th sample held, in phase with the 128-frame quantum | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `coarse-3` | `coarse(3)`: the hold restarts at every quantum's frame 0 (the last hold two frames long) | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `crush-int` | `crush(4)` on exact halves: `Math.round` rounds half toward +infinity (`+0.0625` to 1/8, `-0.0625` to -0) | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `crush-frac` | `crush(3.5)`: a non-dyadic `Math.pow(2, 2.5)`, so the result depends on V8's `pow` | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `shape-basic` | `shape` 0.6 with `shapevol` 0.5 (colon form): the rational shaper and the clamped postgain | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `shape-one` | `shape(1)` clamped to `1 - 4e-10` (a near sign function) | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `distort-rational` | `distort(2)` with types 0 (scurve), 2 (hard), 6 (fold): the closed forms and `k = expm1(2)` | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `distort-tanh` | types 1 (soft) and 3 (cubic): V8's `tanh` and `log1p` | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `distort-diode` | types 4 (diode) and 5 (asym): V8's `cosh`, the bias and the denominator | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `distort-sine-cheb` | types 7 (sinefold) and 8 (chebyshev): V8's `sin` and the 63-step recurrence | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `distort-vol` | `distortvol(0.4)`: the per-sample postgain clamp | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `transient-attack` | `transient(0.8)`: muted until the first quantum after the onset, the envelope followers, `avgGain`, makeup and soft clip | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `transient-sustain` | `transient("0:-0.8")`: the sustain branch and its 36 dB scale | -140 dB (`evidence/engine-null-micro-shapers.json`) |
| `transient-stereo` | `transient` on a `pulse`: two channels, channel-major, `avgGain` carried from channel 0 to 1 | -140 dB ([pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md)) |
| `ladder-const` | `ftype('ladder')`: the 4-pole `fast_tanh` loop, the one-sample FIR latency, the ring cut after the oscillator stops | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `ladder-drive` | ladder `drive(3)`: input gain `clamp(exp(3), 0.1, 2000)` and the `1 / drive` makeup | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `ladder-env` | a lowpass envelope on the ladder's `frequency`: events on a worklet param read once per quantum | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `ladder-hpf` | `hpf` with `ftype('ladder')`: the ladder lowpass at the highpass frequency, resonance from `hresonance` | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `measure-worklet-teardown` | the quantum a ladder loses its input after its oscillator stops: `ceil(stopFrame / 128) * 128` (`evidence/measure-worklet-teardown.json`) | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `djf-lp-hp` | orbit `djf` as lowpass and highpass, the value read at each quantum's first frame, state kept across switches | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `djf-none` | the djf dead zone (0.49 to 0.51): input copied, filter state left stale | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `djf-insert-persist` | the djf inserted at the pause frame of its first hap, `.value` set there, kept for later haps without `djf` | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `djf-returns` | the delay return goes through the orbit djf with the voices | -140 dB (`evidence/engine-null-micro-ladder-djf.json`) |
| `supersaw-unison1` | `supersaw` with one voice: the seeded phase, polyBLEP, the `0.3 / sqrt(voices)` peak, stereo output | -140 dB (`evidence/engine-null-micro-synth.json`) |
| `supersaw-unison5` | five voices: detuner, swapped `sqrt` pan gains, phases in voice order, float `+=` accumulation | -140 dB (`evidence/engine-null-micro-synth.json`) |
| `supersaw-frac-unison` | `unison(2.5)`: three passes with the detuner scaled by `freqspread / 1.5` | -140 dB (`evidence/engine-null-micro-synth.json`) |
| `supersaw-pitch-env` | a pitch envelope on the worklet's a-rate `detune` | -140 dB (`evidence/engine-null-micro-synth.json`) |
| `supersaw-vibrato` | vibrato as a connection into the worklet's `detune`, summed with its timeline | -140 dB (`evidence/engine-null-micro-synth.json`) |
| `supersaw-boundary` | onsets exactly on a quantum boundary: `currentTime <= begin` decided by `f32(begin)` | -140 dB (`evidence/engine-null-micro-synth.json`) |
| `measure-worklet-start` | the quantum a worklet source first runs in: the first with `currentTime > f32(begin)` (`evidence/measure-worklet-start.json`) | -140 dB (`evidence/engine-null-micro-synth.json`) |
| `pulse-basic` | Stereo pulse on two notes, with four-point edge correction and two samples of oscillator delay | -140 dB ([pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md)) |
| `pulse-pw` | Width settings 0.1 and 0.9 preserve the existing spacing of the pulse edges | -140 dB ([pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md)) |
| `pulse-pan-center` | `pan(0.5)` behind a stereo source (stereo law) and a mono one (mono law) | -140 dB ([pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md)) |
| `pulse-fm-vibrato` | pitch envelope, vibrato and FM on a worklet source, pulled from the pause frame | -140 dB ([pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md)) |
| `tremolo-sum` | tremolo: the AM gain's intrinsic `max(1 - depth, 0)` plus the LFO, `tremolosync`, `tremolophase` | -140 dB (`evidence/engine-null-micro-processors.json`) |
| `tremolo-depth-edges` | tremolo depth -0.5 (NaN replaced by the param default), 0, 1 and 2 (LFO clipped) | -140 dB (`evidence/engine-null-micro-processors.json`) |
| `lfo-shapes` | the `lfo` processor's five shapes: the f32 `time` start phase, curve, clamp | -140 dB (`evidence/engine-null-micro-processors.json`) |
| `pulse-pwm` | `pwrate`/`pwsweep`: an `lfo` summed per frame into the a-rate `pulsewidth` | -140 dB ([pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md)) |
| `filter-lfo-lp` | a lowpass filter LFO (`lprate`, `lpdepth`): per-frame frequency, per-sample coefficients | -140 dB (`evidence/engine-null-micro-processors.json`) |
| `filter-lfo-hp-bp` | the highpass and bandpass filter LFO keys | -140 dB (`evidence/engine-null-micro-processors.json`) |
| `filter-lfo-sync-shape` | `lpsync`, `lpshape`, `lpskew`, `lpdc` and the default depth | -140 dB (`evidence/engine-null-micro-processors.json`) |
| `filter-lfo-env` | a filter envelope and a filter LFO on one `frequency`, then the `[0, 22050]` clip | -140 dB (`evidence/engine-null-micro-processors.json`) |
| `compressor-fresh` | a compressor created after the warm-up window: fresh onset state, superdough's defaults (macOS only) | -140 dB (`evidence/engine-null-micro-native.json`) |
| `compressor-warmup` | a compressor's onset state after silence: warm-up through the quantum at 71808, frozen from 71936 (macOS only; `evidence/compressor-warmup.json`) | -140 dB (`evidence/engine-null-micro-native.json`) |
| `compressor-params` | ratio, knee, attack and release clamps, an infinite release rate, the colon form (macOS only) | -140 dB (`evidence/engine-null-micro-native.json`) |
| `compressor-stereo` | the compressor's two output channels, a mono input feeding both, the stereo pan law after it (macOS only) | -140 dB (`evidence/engine-null-micro-native.json`) |
| `vowel-formants` | the vowel bank: five bandpass formants into `Gain(8)` (measured floor -145.12 dB, five summed inputs) | -139 dB (`evidence/engine-null-micro-native.json`) |
| `vowel-unknown` | an unknown vowel: the `hap` error, the voice never connected | -140 dB (`evidence/engine-null-micro-native.json`) |
| `phaser-notch` | the default phaser: the notch, its `lfo` on `detune`, the float `exp2f` detune path (`evidence/phaser-exp2.json`) | -140 dB (`evidence/engine-null-micro-native.json`) |
| `phaser-params` | `phaserdepth` to Q, its clamps, depth 0 (no phaser), `phasercenter`, `phasersweep` | -140 dB (`evidence/engine-null-micro-native.json`) |
| `fxrelease-fade` | `FXrelease` above `release`: hold to `end + release`, linear fade; at or below it, no gain | -140 dB (`evidence/engine-null-micro-native.json`) |
| `duck-first-ramp` | the first duck on an orbit: the first ramp inserts `setValueAtTime(1, pause)` (`evidence/duck-first-ramp.json`) | -140 dB (`evidence/engine-null-micro-duck.json`) |
| `duck-gap-ramp` | ducks after completed recoveries, `duckdepth` 2 floored at 0.01 | -140 dB (`evidence/engine-null-micro-duck.json`) |
| `duck-cancel-hold` | a duck inside the previous recovery: `CancelValues` holding the cached ramp value | -140 dB (`evidence/engine-null-micro-duck.json`) |
| `duck-across-pause` | a recovery crossing pauses, cancelled inside it by a later duck | -140 dB (`evidence/engine-null-micro-duck.json`) |
| `duck-call-order` | two ducks in one pause run in call order | -140 dB (`evidence/engine-null-micro-duck.json`) |
| `duck-multi-target` | `duckorbit("2:3")` with per-target onset and depth arrays | -140 dB (`evidence/engine-null-micro-duck.json`) |
| `bus-sound` | the `bus` send (`busgain`) and the `bus` sound reading it as a stereo source | -140 dB (`evidence/engine-null-micro-buses.json`) |
| `mod-lfo-gain` | an `lfo` modulator on the gain stage: `currentValue`, range, start quantum (`evidence/mod-lfo-gain.json`) | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-lfo-cutoff` | an `lfo` on a lowpass cutoff: the frequency range from `currentValue` | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-lfo-cutoff-24db` | one `lfo` into both biquads of a 24 dB lowpass | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-lfo-pan` | an `lfo` on the panner (`currentValue` 0 read as 1) | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-lfo-note` | an `lfo` on the carrier's `frequency` beside two FM inputs (measured floor -129.45 dB, three summed inputs) | -124 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-lfo-retrig-sync` | `lfo` `sync` and `retrig`: frequency `sync * cps`, the start phase from `time` | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-lfo-of-lfo` | a modulator of a modulator: the second `lfo` on the first one's rate | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-lfo-source-param` | `lfo`s on the worklet synths' `pulsewidth` and `freqspread` | -140 dB ([pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md)) |
| `mod-env-cutoff` | an `env` modulator on a lowpass cutoff (`depthabs`) | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `env-staircase` | the `envelope` processor end to end: the 128-frame staircase, cumulative decay, sustain to `end` | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-orbit-delaytime` | an `lfo` on the orbit delay's `delayTime`: `currentValue` from the last rendered quantum (`evidence/mod-orbit-delaytime.json`) | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `mod-orbit-djf` | an `lfo` on the orbit djf's `value` | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `bus-bmod` | the patched bus modulator (`bmod-gate`): DC offset, depth gain, gate, down-mix into the cutoff | -140 dB (`evidence/engine-null-micro-modulators.json`) |
| `fx-chain-gain` | an FX chain entry's own gain stage and filters, the hap's `ftype('24db')` | -140 dB (`evidence/engine-null-micro-chains.json`) |
| `fx-chain-mod` | modulators addressing an FX entry (`fxi`, the `modulator-fxi-copy` default) | -140 dB (`evidence/engine-null-micro-chains.json`) |
| `fx-chain-delay` | an FX entry's local delay: dry and wet into a stereo sum, cut at `endWithRelease` | -140 dB (`evidence/engine-null-micro-chains.json`) |
| `fx-chain-room` | an FX entry's local reverb: its own seeded IR, cut at `endWithRelease` | -137 dB (`evidence/engine-null-micro-chains.json`) |

## Controls

`controls.md` is the audit of every control key (where superdough reads it, whether the engine
implements it, ignores it like superdough does for synths, or refuses it as unsupported).
`ALLOWED_KEYS` in `src/capability.rs` is exactly its `implemented` and `ignored` rows; a test checks
it. `CHAIN_KEYS` is checked both ways against what an FX entry reads (`parse_entry_params`); `i`
is the one key superdough reads per entry that the engine ignores. The audit's `owner` column
records the implementation scope for each unsupported key and each refused sound. A unit test
checks the coverage of that inventory.

## Live mode

`strudeland_engine::live::LiveEngine` is the streaming driver on the render core the offline
`Engine` uses (`render_core::Core`): stereo at 44100 Hz, one 128-frame quantum per `render_quantum`
call, written interleaved `L R` into the caller's 256-sample buffer. `strudeland-live`'s engine
thread owns it (it is `!Send`).

- **Values and orbits.** `add_value(index, value, orbit)` takes indexes 0, 1, 2, ... in order
  (anything else is a `LiveEvent::Error` and is ignored); the bus graph grows with each value,
  and a voice already playing moves to its value's new bus level from the next quantum. An
  `OrbitSpec` given with a value creates that orbit (creation order is call order; an existing
  number is a no-op), whether or not the value is supported.
- **Haps.** `schedule(LiveHap)` times the voice by `t_sec` as offline does (`frame` is
  `floor(t_sec * 44100)`, informational), with the hap's own `cps`. A hap before the render
  head is **late**: it starts at the head and `counters().late_haps` counts it; it is not an
  error and pushes no event. Support is decided at the first hap of each `(value, cps)`
  (`capability::signature_of_with_bus`, a bus cycle counting as unsupported) and cached; an
  unsupported hap is skipped (`skipped_haps`) and the first of each signature pushes one
  `LiveEvent::Warning`. A hap naming an unknown value is an `Error` event.
- **Events.** `take_events` drains the engine's events, then at most 32 of the core's render
  errors (`RenderError.message` verbatim; the rest of that batch are dropped).
- **`stop`** drops every voice; the orbits' effect tails ring out, and the values, orbits, IR
  cache and counters stay. **`set_seed`** clears the noise and IR caches for the haps that
  follow; support decisions and warnings stay.
- **Reverb IRs.** With no worker attached, or with `LiveOptions { sync_ir: true }` (tests), the
  pending IR requests are built at the next quantum exactly as offline settles them at the end of
  a pause, so a job's haps replayed pause by pause give the offline render bit for bit
  (`tests/live_equals_offline.rs`). With a worker (`attach_ir_worker`), each request is sent as an
  `IrRequest` (one per IR key in flight; a cached key applies at once), the worker runs
  `build_ir`, and `poll_ir` (the first step of `render_quantum`) lands each `IrBuilt` at the next
  quantum boundary; a reverb's newest request wins whatever order results arrive in. Until then
  the previous IR plays, so the **first use of a reverb is silent** for the build time (a default
  room takes milliseconds). A worker that is gone falls back to building in place: for the rest of the settle in which the send failed with `build_ir` (`Convolver::new_live`), and from the next quantum through the synchronous settler (`Convolver::new`). Both drivers
  settle through the core's one walk (`Core::settle_reverbs`): offline and synchronous live
  through the synchronous settler (`Convolver::new`), the worker through its own settler
  (`build_ir`, `Convolver::new_live`).
- **Long live reverbs.** The worker prepares the existing zero-latency stages for the first
  8192 taps. Later 4096-tap partitions share each input transform and accumulate in the frequency
  domain over 32 render quanta. Each attached convolver has independent input and overlap history.
  Cached IRs share the immutable late spectra and reuse prepared FFT plans on installation.
  Summing ahead of the inverse FFT changes floating-point rounding. Offline rendering and
  the synchronous parity path use the Blink stage layout and summation.
- **Seed and IRs.** A request carries the seed in force when it was sent; after a `set_seed` the
  result still lands on the reverbs that asked for it (under the old seed), and the next request
  rebuilds for the new seed (the cache was cleared). Live playback is not byte-reproducible; the
  offline render is the reproducible path.

## Known differences

The engine reproduces Chromium's render of superdough's graph, not superdough's main-thread
behaviour during an offline render, so these differ from a superdough render:

- **Voice disconnect timing.** superdough releases a voice's nodes from its sources' `onended`
  callback, which races Chromium's offline render (it never yields); the disconnect lands a
  varying number of quanta (about 5 to 25) after the source stop. A filter still ringing then
  (a resonant lowpass, a 24 dB cascade) is cut at that quantum in Chromium; the engine keeps the
  voice until Blink's silence propagation (`TailFrame`, 1/32768) or the 1e-10 floor ends it, so
  the resonant tail lasts longer and is the same every run.
- **Other main-thread cuts.** The same cleanup cuts a vowel's formant rings and other filter
  tails at a run-dependent quantum (the new fixtures fade such voices to exactly 0 with `fxr`
  first). The affected historical comparisons varied from run to run. Two fixtures using shape,
  distort and duck measured residuals of −56.9 and −66.3 dB against superdough. The measurements
  could not isolate a cause. See the [effect calibration](../../plans/0011-engine-effects/evidence/calibration.md)
  for the historical worklet checks and the [pulse calibration](../../plans/0011-engine-effects/evidence/pulse-replacement-calibration.md)
  for repeatability of the current pulse oscillator.
- **Main-thread releases the engine fixes at the earliest quantum.** superdough releases a
  supersaw at `end` from the main thread; the engine takes the earliest possible quantum, one after
  the quantum holding `end`, so a ladder after a supersaw loses its input then. An FX chain entry's local delay and reverb tails render the quantum holding
  `endWithRelease` and are cut from the next one, where Chromium cuts them at superdough's later
  `releaseNodes`: with `FXrelease > release` the release gain is 0 there and the cut is exact,
  without it the tail still sounds at the cut. A modulator held by the orbit (an `lfo` on the
  orbit delay's `delayTime` or the orbit `djf`) stays connected until its voice is freed, not
  until `releaseNodes`. The `bus` sound is disconnected at `RoundUp128(ceil(end * sr))`, not at
  its main-thread timeout (the envelope is 0 by then, so any frame of that window sounds the
  same). A worklet source's vibrato and FM modulators are pulled from its scheduling frame (Blink
  renders a worklet's params every quantum) and never stopped by the engine (superdough stops them
  after `end`, when nothing is audible).
- **Orbit djf input.** When the disable cascade reaches the orbit's summing node (it has at most
  one input left) and disables its output, Chromium's djf goes silent; the engine keeps the djf's
  input connected and processing for good, and does not model its input being disconnected when
  the summing node empties. No micro case reaches either.
- **Summing order.** Blink keeps a summing junction's and a param's connections in hash sets, so
  with three or more sounding inputs the order, and the last bit of the sum, is undefined (it
  changes between Chromium runs, and when a disconnect reorders the inputs); the engine adds in
  connection order. The effect is around -150 dB. The same holds for the five vowel formants
  (summed in index order; the floor case `vowel-formants`), a param with an `lfo` beside FM inputs
  (`mod-lfo-note`), a bus's feeders (summed in voice creation order) and the `worklets` fixture,
  whose superdough render is not bit-reproducible either (orbit summing order, about -144 dB; it is
  judged by its envelope).
- **Subnormals.** Chromium's audio thread runs with flush-to-zero; the engine flushes where a port
  needs it (the biquad state, the compressor) but not in the panner or `GainNode`, which shows on
  isolated worklet tails.
- **Platform math.** The compressor's `powf`, `log10f`, `exp` and `sin` and `jsmath`'s `pow` and
  `tanh` are the host C library's, as Chromium's are, so their micro cases and exact tests are
  judged on macOS only (the reference's host). Where Chromium's compiler may turn `powf(10, x)`
  into `__exp10f`, the engine calls `powf`; the two are indistinguishable on the cases.
- **Accelerate reductions.** The IR normalization (`vDSP_svesq`) and the convolver's direct stage
  (`vDSP_conv`) sum in an order that depends on alignment; the engine sums in a fixed order (wet
  path within about 1e-6 relative; reverb micro cases and `fx-chain-room` null at -140 to -153 dB, not -inf).
- **Non-finite audio.** A custom wave that overflows once scaled is a hap error, a voice whose
  output turns NaN or infinite is silenced and reported, and an overflowing mix sample is written
  as 0 and reported; Chromium writes the NaN or infinite samples. An IR longer than `2^26` frames
  is an `ir` error.
- **Orbit keys.** superdough keys orbits by `String(orbit)`; the job carries numbers, so `1` and
  `'x'` share one bus, and a duck target orbit keyed by a non-numeric string cannot be addressed.
- **Control coercions.** Non-number `duck*` values are NaN in the engine where superdough coerces
  a numeric string; a `null` phaser control is treated as absent (JS passes `null` through a
  parameter default); an array or object `distorttype` is unsupported; an unknown vowel naming an
  `Object.prototype` property gets a different message; an `fxi` naming a JavaScript prototype
  property is a `hap` error here and a logged skip in superdough. A NaN `pwsweep` builds the LFO
  and throws, as superdough does.
- **Error kinds.** superdough's logged lines (a missing duck target, a skipped modulator) reach the
  render page as errors classified by their text; the engine reports the same text as `other`,
  so a line whose text contains `worklet`, `processor` or `sample` (a control or `fxi` naming
  one) has a different kind in the two renders. The `unsupported` signature of a modulator target
  (`<type>:<control>`) ignores a throw on a non-finite value before a later noise target.
- **Engine limits.** A supersaw `voices` above 4096 finishes the processor with a `hap` error
  (Chromium would loop, effectively hanging). The engine's IR cache clones a convolver per voice
  for FX chain reverbs (about 35 MB for fx-chain, `calibration.md` section 4). An uncached
  `CancelValues` reached after pruning starts from 0, as Blink's does (unreachable from
  superdough).
- **Envelope processor quirks** (ported as they are; upstream candidates): a hap whose `begin` is
  exactly 0 never triggers; the decay and release use the cumulative time since the trigger; the
  output is a 128-frame staircase (`currentTime` is constant within a quantum); superdough sets
  `end` to both the gate and the release start, so the release never sounds.
- **Bus cycles.** A bus graph with a cycle is refused (`bus-cycle`). Chromium breaks a cycle with
  a pull-order-dependent one-quantum delay, not worth porting; the check is conservative (a `bmod`
  that would skip still counts as a read).
- **superdough errors that are not modelled.** The noise-mix teardown that throws
  `InvalidAccessError` when such a voice ends inside a render has no engine
  counterpart; the engine renders the voice normally.
- **Not ported.** `setTargetAtTime`, `setValueCurveAtTime` and `cancelScheduledValues`
  (superdough never calls them offline). `controls.md` lists unsupported controls and sounds,
  including samples and soundfonts, sample IRs (`ir`, `irspeed`, `irbegin`), `stretch`,
  wavetables (`wt_*`), `bytebeat`, kabelsalat (`workletSrc`, `workletInputs`), the `source` function
  key, `sbd` (Blink's `WaveShaperNode`) and zzfx (`zzfx`, `z_*`). The engine refuses such
  jobs with `unsupported` errors. The `signatures` subcommand lists them.

## Re-porting after a Playwright bump

1. Read the Chromium version Playwright bundles (`node_modules/playwright-core/browsers.json`).
2. Fetch each Blink file listed above at the old and the new tag and diff them; port the changes.
3. Update the tag here, in every ported file's header and in `tests/licence.rs` (the V8 revision
   too: `src/jsmath/v8.rs` and the V8 family header there; re-check which V8 and host functions
   `Math.*` calls at the new revision, `src/jsmath/mod.rs`).
4. Regenerate the references (`bun run engine:refs --write`, see References; this re-evaluates
   `v8-math.json`) and the vectors (`bun run engine:vectors --write`), then run `cargo test -p
   strudeland-engine` (`tests/v8_math.rs` must still match every vector).
5. Re-run the calibration (`bun scripts/render/probes/engine-null.probe.ts`) and
   `bun run render:check --engine rust`; when a micro case misses its threshold, fix the port
   before touching the threshold, and recalibrate `PROFILES.engine` only from new evidence.
