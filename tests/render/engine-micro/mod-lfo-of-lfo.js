// Micro case: a modulator of a modulator. `lpf(500).lfo()` is lfo 0 on the cutoff (rate 1,
// depth 500, range { -480, 23500 }); the second `lfo({ sc: 'rate', da: 3 })` defaults its
// control to `lfo_0` and its subControl rate maps to lfo_0's frequency (currentValue the f32
// rate 1, depthabs 3, getLfo's min/max -1.5 and 1.5). The second lfo is rendered before the
// first, which reads it through its frequency param. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2").s('sawtooth').lpf(500).lfo().lfo({ sc: 'rate', da: 3 })
