// Micro case: distort(2) with distorttype 7 (sinefold) and 8 (chebyshev), one voice per 0.25 s
// bar: V8's sin and the 63-step Chebyshev recurrence. Two bars, 0.5 s.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("a3").s('sawtooth').distort(2).distorttype("<7 8>")
