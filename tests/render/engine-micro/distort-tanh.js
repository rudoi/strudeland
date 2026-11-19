// Micro case: distort(2) with distorttype 1 (soft) and 3 (cubic), one voice per 0.25 s bar:
// V8's tanh and log1p. Two bars, 0.5 s.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("a3").s('sawtooth').distort(2).distorttype("<1 3>")
