// Micro case: an unknown vowel letter throws `vowel: unknown vowel zz` in the FX loop: a hap
// error, the voice is never connected and makes no delay send. The first half's sawtooth (no
// vowel) creates the orbit delay, which rings on untouched. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('sawtooth').delay(0.5).delaytime(0.04)
$: note("~ a3").s('sawtooth').vowel("zz").delay(0.5).delaytime(0.04)
