// Micro case: an lfo on a lowpass cutoff (`lpf(800).lfo({ r: 4, dep: 0.5 })`): currentValue 800
// >= 30 gives the range { min: 20 - 800, max: 24000 - 800 }, depth 0.5 * 800, the LFO summed into
// the biquad's frequency per frame (per-sample coefficients from scheduling). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').lpf(800).lfo({ r: 4, dep: 0.5 })
