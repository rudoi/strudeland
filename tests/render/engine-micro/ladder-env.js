// Micro case: a lowpass envelope on the ladder's frequency param (lpenv(4), lpattack(0.05)): the
// exponential ramps are events on a worklet param, read once per quantum at its first frame (a
// 128-frame staircase). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').lpf(300).lpq(8).ftype('ladder').lpenv(4).lpattack(0.05).release(0.02)
