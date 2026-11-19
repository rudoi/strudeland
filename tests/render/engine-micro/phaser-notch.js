// Micro case: the default phaser on a sawtooth: a notch at 1000 + 282 Hz with Q 0.5 (depth 0.75)
// and the lfo worklet (a triangle, +-2000 cents, 4 Hz) connected to its detune, so every quantum
// the LFO runs has 128 coefficient sets; the LFO is silent in the onset quantum. Pins the exp2
// overload of NormalizeFrequency's detune path. One 0.5 s bar, held past the end.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('sawtooth').phaserrate(4).clip(8)
