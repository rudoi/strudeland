// Micro case: a constant bandpass (linear Q) with bandq given and with the default Q 1. The second
// voice sits inside the first pattern's second note, so at most two voices sound at once. One
// 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').bpf(1000).bpq(4)
$: note("~@5 a3 ~@2").s('square').bpf(500)
