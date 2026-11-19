// Micro case: attack only (0.1 s); getADSRValues gives sustain 1, decay and release their
// minimums. One held sine note in one 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('sine').attack(0.1)
