// Micro case: supersaw with unison(2.5): the k-rate voices param is fractional, so the voice loop
// runs ceil(2.5) = 3 passes with the detuner scale freqspread / 1.5, and the envelope peak is
// 0.3 * (1 / sqrt(2.5)). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('supersaw').unison(2.5)
