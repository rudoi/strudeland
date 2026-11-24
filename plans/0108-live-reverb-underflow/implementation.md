# Live reverb implementation

The existing long convolver transforms each late partition's input separately and applies a
separate inverse transform. A live render profile with fixed rooms showed the render thread
continuously busy while the IR worker was asleep. The bypass control left time for the render
thread to wait for the next refill. Room changes add setup work, and a warm cache hit could
clone a large convolver and trigger FFT preparation in its first quantum.

The [live tail](../../crates/strudeland-engine/src/reverb/tail.rs) shares input transforms,
sums late partitions over the next 32 quanta, then overlap-adds the inverse transform when the
output is due. The first 8192 taps retain the existing early stages. A cached key shares late IR
spectra and FFT plans while obtaining an independent history. Stale worker results skip
instantiation. The synchronous reference path uses the Blink stage layout.

## Public synthetic evidence

On Apple Silicon, a one-orbit timing probe with a long synthetic IR measured a 5.49 ms median
four-quantum slice on the original convolver and 0.10 ms with grouped tails. A warm simulated room
replacement measured 12.04 ms before and 0.12 ms with the new path. This timing probe measured the
convolver in isolation across several partition periods. The real-time gate exercises full histories
with the production worker, player, ring, and 48 kHz resampler.

Four simultaneous synthetic orbits were the first failure in the orbit-count sweep for the
chosen room size. Three sustained tones passed. Four produced 457 underruns and reached 0 ms of
fill in a seven-second baseline run. The regression requires all four wet tones during startup
and sustained playback, zero playback and scheduling failures, and at least one callback of
minimum fill. The test waits for stats covering the final measurement interval.
The release [regression](../../crates/strudeland-live/tests/dropout.rs) passed with zero
underruns, zero late or skipped haps, and 32.02 ms of minimum fill.

The direct-convolution tests cover the transition into the grouped tail, stereo signals with
distinct histories, short final partitions, and output through the tail. A warm A-to-B-to-A
room change tests immediate cached application after out-of-order worker results.
