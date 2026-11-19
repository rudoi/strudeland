// Micro case: postgain (the last Gain of the chain) and dry (a Gain into the orbit instead of the
// direct connection), with and without pan. The panned triangle starts after the first voice has
// ended, so at most two voices sound at once. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('sawtooth').postgain(0.5).dry(0.7)
$: note("~ e4").s('sawtooth').postgain(2)
$: note("~@9 c4@6 ~").s('triangle').pan(0.2).dry(0.4).postgain(1.5)
