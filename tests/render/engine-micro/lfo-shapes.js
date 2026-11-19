// Micro case: the lfo processor's waveshapes through tremolo(10) on five 0.1 s notes, one shape
// each (tremoloshape "tri sine square ramp saw" maps to 0 1 4 2 3, skew 0.5 since a shape is
// given): the start phase ffrac(time * frequency) from the float32 time = cycle / cps, the
// frequency read once per quantum, curve 1.5 and the [0, 1] clamp; depth 1 makes the onset
// quantum silent. legato(0.9): at most two voices overlap (a release under the next onset). One
// 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 a3 a3 a3 a3").s('triangle').legato(0.9).tremolo(10).tremoloshape("tri sine square ramp saw")
