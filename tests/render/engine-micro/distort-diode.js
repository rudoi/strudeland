// Micro case: distort(2) with distorttype 4 (diode) and 5 (asym), one voice per 0.25 s bar: V8's
// cosh, the bias and the denominator. Two bars, 0.5 s.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("a3").s('sawtooth').distort(2).distorttype("<4 5>")
