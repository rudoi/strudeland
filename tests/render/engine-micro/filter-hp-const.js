// Micro case: a constant 12 dB highpass with resonance at three pitches. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 a3 a4").s('sawtooth').hpf(1200).hpq(8)
