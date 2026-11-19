// Micro case: tremolo depth edges on a constant source, four 0.125 s notes: depth -0.5 (intrinsic
// 1.5, the LFO's pow(negative, 1.5) is NaN and the gain param replaces the sum by its default 1),
// depth 0 (gain 1, the LFO outputs 0), depth 1 (gain 0: silent in the onset quantum, before the
// LFO's first run) and depth 2 (the LFO clipped flat at 1). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("one one one one").sustain(1).tremolo(16).tremolodepth("-0.5 0 1 2")
