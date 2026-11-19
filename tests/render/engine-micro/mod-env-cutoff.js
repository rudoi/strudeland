// Micro case: an env modulator on a lowpass cutoff (`lpf(300).env({ a, d, s, da: 2000 })`):
// depthabs 2000, the range from currentValue 300 ({ min: -280, max: 23700 }), attack 0.05,
// decay 0.1, sustain 0.3, end = endWithRelease, summed into the biquad's frequency. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2").s('sawtooth').lpf(300).env({ a: 0.05, d: 0.1, s: 0.3, da: 2000 })
