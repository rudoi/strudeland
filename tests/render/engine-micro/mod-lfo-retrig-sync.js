// Micro case: lfo `sync` and `retrig` on a lowpass cutoff in two cycles: frequency = sync * cps
// (1.5 * 4 = 6 Hz), and the start phase from time = cycle / cps (0 in cycle 0, 0.25 s in cycle
// 1: phase 0.5) without retrig, time 0 with `rt: 1` (phase 0 in both cycles). legato(0.9): the
// notes of one line never overlap, so at most two voices sound at once. Two 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("<a3 e3>").s('sawtooth').legato(0.9).lpf(600).lfo({ s: 1.5 })
$: note("<e3 a3>").s('sawtooth').legato(0.9).lpf(600).lfo({ s: 1.5, rt: 1 })
