// Micro case: one pan value per hap across the range (k-rate after the pause quantum: float
// gains), including values outside [0, 1] that the pan param clamps. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("c4*8").s('sawtooth').pan("-0.5 0 .1 .35 .5 .7 .9 1.5")
