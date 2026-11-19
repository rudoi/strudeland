// Micro case: a lowpass filter LFO (createFilter): lprate(4).lpdepth(0.5) on lpf(800) connects an
// lfo (triangle, dcoffset -0.5, depth 0.5 * 800, min -770, max 19200, time = cycle / cps, end
// the hap end) to the biquad's frequency: the sum per frame, per-sample coefficients. One 0.5 s
// bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').lpf(800).lprate(4).lpdepth(.5)
