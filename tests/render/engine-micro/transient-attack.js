// Micro case: transient(0.8) on a triangle with attack(0): the processor is muted until the first
// quantum whose start time is after the onset (double begin), then the f32 envelope store per
// block, avgGain, the makeup gain and the soft clip. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('triangle').attack(0).transient(0.8)
