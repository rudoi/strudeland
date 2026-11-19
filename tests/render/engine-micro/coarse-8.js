// Micro case: coarse(8) on a sawtooth: the worklet holds every 8th input sample, in phase with
// the 128-frame quantum (128 is a multiple of 8). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').coarse(8)
