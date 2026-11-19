// Micro case: the default amp envelope (no attack, decay, sustain or release given) on one held
// sine note: attack, decay to the default sustain, hold, release. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('sine')
