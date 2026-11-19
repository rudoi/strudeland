// Micro case: transient("0:-0.8") (transsustain -0.8): the sustain branch with its 36 dB scale.
// One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('triangle').transient("0:-0.8")
