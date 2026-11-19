// Micro case: lowpass filter envelopes (exponential ramps on the frequency param: per-sample
// coefficients), a positive lpenv with its ADSR and a negative lpenv with fanchor. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a2 ~ e3 ~").s('sawtooth').lpf(300).lpq(4).lpenv(4).lpattack(0.05).lpdecay(0.05).lpsustain(0.3).lprelease(0.05)
$: note("~ c3").s('square').lpf(2000).lpenv(-3).fanchor(0.5)
