// Micro case: exponential ramps whose ends have opposite signs (pitch envelope -600 to 600 cents,
// pcurve 1, panchor 0.5): Blink holds the start value instead of ramping. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 a4").s('triangle').penv(12).pcurve(1).panchor(0.5).pattack(0.1)
