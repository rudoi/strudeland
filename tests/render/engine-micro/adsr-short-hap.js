// Micro case: attack + decay longer than the hap (0.03 + 0.2 s, 0.0625 s haps): the decay ramp
// is cut at the hap end at the decay line's value. Onsets at 1/8 bar (2756.25 frames) fall
// between frames. The constant source `one` exposes the envelope. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("~ one ~ ~ one ~ ~ ~").attack(0.03).decay(0.2).sustain(0.2).release(0.02)
