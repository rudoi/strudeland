// Micro case: distort(2) with distorttype 0 (scurve), 2 (hard) and 6 (fold), one voice per
// 0.15 s bar: the closed forms and k = expm1(2). Three bars, 0.45 s.
const meta = { bpm: 400, meter: '1/4', sections: [{ name: 'a', bars: 3 }] };

$: note("a3").s('sawtooth').distort(2).distorttype("<0 2 6>")
