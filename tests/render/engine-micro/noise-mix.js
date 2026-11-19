// Micro case: the noise mix on oscillators (drywet with the cached pink buffer, both started at
// t): noise 0.3 (dry 1, wet 0.6), 0.8 (dry 0.4, wet 1) and 1.2 (dry -0.4, wet 1). release(8)
// keeps every voice sounding past the window: superdough's teardown of an ended noise-mix voice
// can throw (the pink source is released before the drywet disconnect). Orbit 1 holds two
// voices, orbit 2 one, so no summing junction ever has more than two sounding inputs (the sum
// order of three is not fixed in Chromium). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 e3").s("sawtooth").noise("0.3 0.8").release(8)
$: note("~ c4").s("sawtooth").noise(1.2).release(8).orbit(2)
