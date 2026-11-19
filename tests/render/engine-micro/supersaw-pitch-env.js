// Micro case: supersaw with penv(12).pattack(0.1): the pitch envelope's events on the worklet's
// a-rate detune param (128 values per quantum), applied as freq * 2^((detune / 100) / 12). One
// 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('supersaw').penv(12).pattack(0.1)
