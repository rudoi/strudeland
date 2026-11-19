// Micro case: chained FM with envelopes: modulator 2 into modulator 1 (fmi2) into the carrier
// (fmi), a linear envelope on modulator 1 (fmenv 'lin') and the default exponential one on
// modulator 2; then modulator 3 (a square, fmi31) into modulator 1. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~ ~ ~").s('sine').fm(1.5).fmi2(2).fmh(2).fmh2(0.5).fmattack(0.05).fmdecay(0.1).fmsustain(0.3).fmenv('lin').fmdecay2(0.2)
$: note("~ ~ e3 ~").s('sawtooth').fm(1).fmi31(2).fmwave3('square').fmh3(4)
