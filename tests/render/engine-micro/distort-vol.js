// Micro case: distort(2).distortvol(0.4): the per-sample postgain clamp(0.4, 0.001, 1) times the
// default scurve. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('sawtooth').distort(2).distortvol(0.4)
