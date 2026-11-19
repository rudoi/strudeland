// Micro case: ftype '24db' and ftype 2 build two cascaded biquads, each with its own envelope.
// The second voice sits inside the first pattern's second note, so at most two voices sound at
// once (Chromium's summing order of three or more inputs is not fixed). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 e3").s('sawtooth').lpf(600).lpq(3).ftype('24db')
$: note("~@5 a3 ~@2").s('sawtooth').hpf(400).lpf(5000).ftype(2).lpenv(2)
