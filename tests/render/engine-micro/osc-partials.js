// Micro case: custom periodic waves from waveformN: a partial count (`n`, magnitudes 1) on a
// sawtooth and a magnitude list (`partials`) on a square; createPeriodicWave's band-limited
// tables and normalization. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 a4 ~ ~").s('sawtooth').n(12)
$: note("~ ~ e4 e5").s('square').partials([1, 0.5, 0, 0.25, 0.8])
