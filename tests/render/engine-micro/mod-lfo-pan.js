// Micro case: an lfo on the panner (`pan(0.5).lfo()`): `pan.value = 2 * 0.5 - 1 = 0`, so
// currentValue 0 reads as 1 (depth 1, min -0.5, max 0.5, rate 1), the a-rate pan from
// scheduling and its [-1, 1] clip. Two 0.25 s notes. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 e3").s('sawtooth').pan(0.5).lfo()
