// Micro case: FM pairs (fmi into the carrier's frequency, modfreq = carrier frequency.value *
// fmh): the modulator starts with start() at the pause frame, so it first plays in the quantum
// the carrier first evaluates its params, and the carrier runs its a-rate path. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~ e4 ~").s('sine').fm(2).fmh(1.5)
$: note("~ c4 ~ g3").s('triangle').fm(0.7).fmh(3)
