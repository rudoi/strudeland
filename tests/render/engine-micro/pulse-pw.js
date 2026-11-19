// Micro case: widths 0.1 and 0.9 give negative intervals of 45% and 5% of each cycle. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 a3").s('pulse').pw("0.1 0.9")
