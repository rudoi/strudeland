// Measure item: the quantum a ladder filter loses its input after its oscillator stops. One
// sawtooth through a self-oscillating ladder (lpq 40: k = 5.2 > 4), a 0.25 s hap (legato 0.5 of a
// 0.5 s step) with release 0.05 in a 1 s bar, so the oscillator stops mid-bar and the ring is
// large when the cut comes. `engine:refs --write` records the last non-zero frame against the
// rule (output zero from ceil(stopFrame/128)*128, stopFrame = TimeToSampleFrame(t + duration +
// release + 0.01, RoundUp)).
const meta = { bpm: 240, meter: '4/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 ~").s('sawtooth').lpf(300).lpq(40).ftype('ladder').legato(0.5).release(0.05)
