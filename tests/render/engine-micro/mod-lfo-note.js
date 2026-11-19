// Micro case (measured floor): an lfo on the carrier's frequency (`lfo({ c: 'note', r: 6, dep:
// 0.02 })`) of a sine with two FM modulators into the carrier (fmi with fmh 1.5, fmi20 with fmh2
// 2): three sounding inputs on one frequency param, which Blink sums in hash-set order (the engine
// sums the FM connections in matrix order, then the lfo). currentValue is the f32 frequency of
// a3 (220), so the range is { -200, 23780 } and the depth 0.02 * 220. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('sine').fmi(1).fmh(1.5).fmi20(1).fmh2(2).lfo({ c: 'note', r: 6, dep: 0.02 })
