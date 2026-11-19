// Micro case: ladder drive(3): the input gain clamp(exp(3), 0.1, 2000) and the 1 / drive makeup.
// One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').lpf(900).lpq(8).ftype('ladder').drive(3).release(0.02)
