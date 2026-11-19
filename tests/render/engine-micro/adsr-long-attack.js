// Micro case: attack longer than the hap (0.4 s attack, 0.25 s hap): getParamADSR ramps to the
// attack line's value at the hap end, then releases. The constant source `one` exposes the
// envelope itself. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("one ~").attack(0.4).release(0.1)
