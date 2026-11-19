// Micro case: oscillators whose onsets fall between frames (odd eighths at 2756.25-frame steps)
// in quanta after the pause quantum: the k-rate first quantum starts at the sub-sample phase
// -offset * incr. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("~ a4 ~ e5 ~ c6 ~ ~").s('sine')
$: note("~ ~ a3 ~ ~ ~ e4 ~").s('sawtooth')
