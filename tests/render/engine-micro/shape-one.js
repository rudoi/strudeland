// Micro case: shape(1): shape >= 1 is clamped to 1 - 4e-10, giving s of about 5e9 (a near sign
// function through the rational shaper). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('sawtooth').shape(1)
