# Local patches to the vendored Strudel

Each entry is one local change to `vendor/strudel/`: what it does, whether to send it upstream, and
every file it modifies or adds (paths relative to `vendor/strudel/`). The integrity test fails if a
vendored file differs from `upstream-manifest.json` without being listed here.

## hap-tap: tap every scheduled hap with its route
Upstream: candidate
`repl({ onScheduleTap })` is called once per scheduled hap, before routing, as
`onScheduleTap(hap, t, cps, route)` with route `default`, `pattern` or `both` (derived from the
hap's own `onSchedule` and `dominantSchedule`). Its errors are logged and never block output.
Opt-in, no change when unset.
```files
packages/core/repl.mjs
```

## audio-rng: seedable random numbers in sound generation
Upstream: candidate
`rng.mjs` (new, imports nothing) gives every random use in sound generation its own mulberry32
stream, seeded from a hash of the audio seed (`setAudioSeed`) and a key naming the use: noise
buffers (cache emptied on a seed change; crackle per voice, also as an FM modulator), reverb IRs
per room settings (an orbit's IR is regenerated after a seed change), zzfx and the supersaw and
wavetable worklet phases per voice (`value._voiceSeed`). `getWorkletFromPool` in `nodePools.mjs`
hands the seed (and the wavetable frames) to a new worklet node in `processorOptions`, which reaches
the processor's constructor synchronously, and to a pooled one in an `initialize` message, which is
not ordered against the render thread. Opt-in: with no audio seed every site keeps `Math.random`.
```files
packages/superdough/nodePools.mjs
packages/superdough/rng.mjs
packages/superdough/noise.mjs
packages/superdough/reverbGen.mjs
packages/superdough/superdoughoutput.mjs
packages/superdough/helpers.mjs
packages/superdough/zzfx.mjs
packages/superdough/zzfx_fork.mjs
packages/superdough/superdough.mjs
packages/superdough/synth.mjs
packages/superdough/wavetable.mjs
packages/superdough/worklets.mjs
```

## rng-mode-getter: read the RNG mode useRNG set
Upstream: candidate
`getRNGMode()` returns the mode `useRNG` last set (`legacy` by default), so a host that resets the
mode before an evaluate can restore it when the evaluate fails.
```files
packages/core/signal.mjs
```

## node-pool-toggle: turn node pooling off
Upstream: candidate
`setNodePooling(enabled)` in `nodePools.mjs` (returns the previous setting). Off: `getNodeFromPool` always
builds a new node and `releaseNodeToPool` only disconnects; turning it off empties the pool. Nodes
return to the pool from `ended` callbacks, so which ones are reusable depends on when those events
reach the main thread; an offline render turns pooling off to stay deterministic. On by default.
```files
packages/superdough/nodePools.mjs
```

## reverb-latest-ir: the latest IR request wins
Upstream: candidate
An orbit's reverb regenerates its IR whenever a hap asks for other room settings, and generated IRs
render asynchronously (a nested `OfflineAudioContext`). They can finish in any order, so with two
requests in flight the convolver kept whichever finished last. `generate` now numbers its requests and
a generated buffer is set only if it was requested after the buffer in place, so the latest request
wins and a stream of changes still applies each IR that finishes in order.
```files
packages/superdough/reverb.mjs
```

## modulator-fxi-copy: default fxi on a copy of the modulator config
Upstream: candidate
`superdough()` wrote `params.fxi ??= key` into the `lfo`/`env`/`bmod` config object of the hap
value. The pattern `modulate` shares that object between haps, so a later hap's `voiceSeed`
hashed the written `fxi` (an interned engine job holds one control map per distinct value and
cannot express that). The three modulator calls now receive `{ ...params, fxi: params.fxi ?? key }`
and the config is never written. Line numbers of `superdough.mjs` are unchanged.
```files
packages/superdough/superdough.mjs
```

## bmod-gate: connect bus modulators through a gate instead of a timeout
Upstream: candidate
`connectBusModulator` connected its modulator to the target params from a `webAudioTimeout`
callback at `begin`, which an offline render runs a varying number of render quanta late. It now
connects `modulator` to the targets at once through a `Gain(0)` gate that opens at `begin` and
closes at `end`. Roadmap #50.
```files
packages/superdough/modulators.mjs
```

## fast-fraction: number-backed rational time
Upstream: strudeland-only
fraction.js 5.3.4 does all arithmetic in BigInt, and that constant cost dominated pattern querying
on the scheduler tick. `fraction.mjs` now returns a `FastFraction`: `s`, `n` and `d` are numbers
(lowest terms, zero is `0/1` with `s = 1`) and `big` holds a fraction.js value when `n` or `d` does
not fit `Number.MAX_SAFE_INTEGER`. Every operation is exactly fraction.js's: an intermediate above
`MAX_SAFE_INTEGER`, a `big` operand, an argument that is not a `FastFraction` or a safe integer
(floats keep fraction.js's Farey parse), `gcd`/`lcm` on an argument not known to be reduced, and
every error case compute with fraction.js and wrap the result. `isFraction` accepts both classes,
`fraction._original` is fraction.js, and `toString`/`toFraction` delegate to it; `ceil` and
`simplify` are not provided (nothing calls them). Checked against BigInt by
`tests/fraction-fast.test.ts`.
```files
packages/core/fraction.mjs
```

## pulse-blep: replace the pulse oscillator
Upstream: candidate
The independently written pulse integrates a cubic B-spline around both fractional edges.
It includes motion of the pulse-width edge, retains the control mapping and stereo routing,
and introduces two samples of delay inside the oscillator.
```files
packages/superdough/worklets.mjs
```

## euclidean-source: port the inner rhythm algorithm from HMT 0.20
Upstream: candidate
The bounded port names its release and GPL-3.0-only terms. The Strudel patterns retain
their original sequence, starting phase, rotations, and handling of inverted rhythms.
```files
packages/core/euclid.mjs
```

## embedded-source-notices: document DSP lineage
Upstream: candidate
Identify the reverbGen changes and the upstream source of the vowel, ladder and bytebeat
processors in the files that carry them. Date the WebDirt vowel adaptations in their notices.
```files
packages/superdough/reverbGen.mjs
packages/superdough/vowel.mjs
packages/superdough/worklets.mjs
```

## mixed-meter-phase: pass elapsed time separately from musical cycles
Upstream: candidate
An optional sixth argument to `superdough` supplies elapsed layout seconds for fixed-Hz
filter LFOs, tremolo and modulation. Beat-synced rates keep their musical phase and the
logical cycle continues to identify each seeded voice. Omitted elapsed time defaults to
the original cycle divided by CPS.
```files
packages/superdough/superdough.mjs
packages/superdough/helpers.mjs
packages/superdough/modulators.mjs
```
