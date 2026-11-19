// Micro case: brown noise (a leaky integrator reading back its float32 output) from meta.seed
// -3, two overlapping voices at 2756.25-frame offsets. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', seed: -3, sections: [{ name: 'a', bars: 1 }] };

$: s("~ brown ~ ~ brown ~ ~ ~").gain(2).release(0.1)
