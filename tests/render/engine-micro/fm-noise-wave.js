// Micro case: noise modulators (looping noise buffers started at the pause frame, density 2 for
// crackle): white noise into the carrier, crackle into a sine modulator, and a sine modulator
// into a noise modulator (no frequency param: the connection is skipped, superdough only warns,
// the sine still runs unheard). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~ ~ ~").s('sine').fm(0.3).fmwave('white')
$: note("~ e4 ~ ~").s('sine').fm(1).fmi2(3).fmwave2('crackle').fmh(2)
$: note("~ ~ c4 ~").s('triangle').fm(0.5).fmwave('brown').fmi2(1)
