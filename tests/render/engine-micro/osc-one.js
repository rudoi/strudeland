// Micro case: `one`, a ConstantSourceNode(1): it starts on the next whole frame (no sub-sample
// phase); the second onset falls at 8268.75 frames. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("one ~ ~ one ~ ~ ~ ~").decay(0.05).sustain(0.5)
