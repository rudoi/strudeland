// Micro case: an exponential pitch envelope with a negative depth (penv -12, pcurve 1): min 1200
// cents, max 0 mapped to 0.001. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 a4").s('sine').penv(-12).pcurve(1).pattack(0.08).pdecay(0.05)
