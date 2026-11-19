// Micro case: stereo (panned) voices into the orbits' summing nodes and their pan-0 stereo
// panners (L = L + R * cos(pi/2), R = R), on two orbits, with and without dry. At most two
// voices overlap per orbit so the summing order cannot change a bit. One 0.5 s bar, mix.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("c4 e4").s('sawtooth').pan("0 1").orbit(2)
$: note("g3 ~ b3").s('triangle').pan(0.3).dry(0.6).orbit(3)
