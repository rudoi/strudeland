// Micro case: crush(3.5): x = Math.pow(2, 2.5) is not dyadic, so the multiply and divide round and
// the result depends on V8's pow. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('sine').crush(3.5)
