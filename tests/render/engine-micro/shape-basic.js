// Micro case: shape 0.6 with shapevol 0.5 (the colon form; Strudel has no shapevol function):
// s = 2 * 0.6 / (1 - 0.6) and the rational shaper, then the postgain clamp
// max(0.001, min(1, 0.5)). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('sawtooth').shape("0.6:0.5")
