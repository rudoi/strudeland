// Micro case: two pitches at the default pulse width, with identical stereo channels. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 a4").s('pulse')
