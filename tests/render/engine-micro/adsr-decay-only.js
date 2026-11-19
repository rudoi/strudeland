// Micro case: decay only (0.1 s), no attack or sustain: getADSRValues' sustain becomes 0.001
// (a decay-to-silence pluck). One sine note in one 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('sine').decay(0.1)
