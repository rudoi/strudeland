// Micro case: the env modulator end to end on the gain stage of a constant source
// (`gain(0.5).env({ a: 0.1, d: 0.1, s: 0.5 })`, depth 1 * 0.5): the envelope-processor's value
// is computed from currentTime once per quantum (a 128-frame staircase), the decay starts at the
// cumulative attack time, then the sustain holds to end = endWithRelease. One 0.5 s note. One
// 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("one").gain(.5).env({ a: .1, d: .1, s: .5 })
