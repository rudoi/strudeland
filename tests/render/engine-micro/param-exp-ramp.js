// Micro case: exponentialRampToValueAtTime (Blink's double accumulator and snap to the target),
// through a pitch envelope with pcurve 1 and panchor 0 (0.001 to 1200 cents and back). One 0.5 s
// bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 a4").s('sine').penv(12).pcurve(1).panchor(0).pattack(0.1).pdecay(0.05).psustain(0.5)
