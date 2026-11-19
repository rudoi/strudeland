// Micro case: waveformN's phase rotation (`phases`, in turns, computed in double and stored as
// float) on a five-partial sawtooth. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 e4").s('sawtooth').partials([1, 1, 1, 1, 1]).phases([0, 0.25, 0.5, 0.125, 0.9])
