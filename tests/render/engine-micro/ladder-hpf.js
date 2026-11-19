// Micro case: hpf(800) with ftype('ladder') builds the ladder lowpass processor at the highpass
// frequency (the processor has no type; resonance from hresonance, default 1). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').hpf(800).ftype('ladder')
