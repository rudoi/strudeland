// Micro case: pulse width modulation: pwrate(4).pwsweep(0.5) builds an lfo (triangle, skew 0.5,
// dcoffset -0.5, depth 0.5, time = begin, min -0.25, max 0.25) whose output is a connection
// summed per frame into the a-rate pulsewidth param (0.5). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3").s('pulse').pwrate(4).pwsweep(0.5)
