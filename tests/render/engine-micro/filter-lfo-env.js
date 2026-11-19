// Micro case: a filter envelope and a filter LFO on one frequency param: lpenv(2) schedules the
// exponential ramps (the timeline), lprate(3).lpdepthfrequency(300) connects an lfo of depth 300
// (depthfrequency replaces depth * frequency); the timeline value plus the input per frame, then
// the [0, 22050] clip. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2").s('sawtooth').lpf(500).lpenv(2).lprate(3).lpdepthfrequency(300)
