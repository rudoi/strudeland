// Micro case: two orbits in mix mode (both on channels 0 and 1, summed at the merger), one mono
// and one panned. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("c3*4").s('sawtooth').decay(0.05).sustain(0.2).orbit(1)
$: note("e4 g4").s('square').gain(0.5).pan(0.8).orbit(2)
