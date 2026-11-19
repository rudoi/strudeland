// Micro case: numeric ftype: 1 is a plain 12 dB biquad (not the ladder, which only the string
// 'ladder' selects), 4 is 12 dB (4 mod 3 = 1) and 5 is 24 dB (5 mod 3 = 2). The 24 dB voice
// comes last: its two resonant biquads ring long enough after the source stop that superdough's
// disconnect (which lands at a quantum that varies from run to run) would cut the ring inside
// the window. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3 a3 c4").s('sawtooth').lpf(700).lpq(10).ftype("1 4 0 5")
