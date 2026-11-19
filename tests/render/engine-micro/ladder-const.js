// Micro case: the ladder filter (ftype('ladder')) on two short sawtooth notes: the 4-pole
// fast_tanh loop, the one-sample FIR latency, k = 8 * 0.13, and the ring cut in the quantum
// after the first oscillator's stop (the cascade disables the ladder's input). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').lpf(900).lpq(8).ftype('ladder').release(0.02)
