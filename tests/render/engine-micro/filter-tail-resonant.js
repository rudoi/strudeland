// Micro case: resonant lowpass filters ringing on after their amp envelopes reach 0 and past
// their sources' stop: the voice keeps processing its chain until the biquad's input has been
// silent for its tail time (Blink's 1/32768 impulse-response threshold), when the filter starts
// propagating silence. The tails (about 3.7 and 3.6 quanta) end a few quanta after the stop,
// before superdough's main-thread disconnect (a race, 6 or more quanta after the stop in
// Chromium) can cut them. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 ~ ~ ~").s('sawtooth').lpf(1500).lpq(15).release(0.01)
$: note("~ ~ a3 ~").s('square').lpf(3000).lpq(20).release(0.01)
