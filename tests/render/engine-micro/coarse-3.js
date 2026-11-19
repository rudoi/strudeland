// Micro case: coarse(3): the hold pattern restarts at frame 0 of every quantum (128 mod 3 = 2),
// so the last hold of a quantum is two frames long. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').coarse(3)
