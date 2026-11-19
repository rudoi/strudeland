// Micro case: the bus send and the bus sound. A DC voice (gain 0.5, the synth envelope, dry 0 so
// its orbit path is silent) sends post * busgain 0.7 to bus 1; s('bus').n(1) reads bus 1 as a
// stereo source through envGain (ADSR defaults [0.001, 0.05, 1, 0.01], linear, no Gain(0.3), no
// source start) into the orbit. Both haps cover the first half of the one 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("one ~").gain(.5).bus(1).busgain(.7).dry(0)
$: s("bus ~").n(1)
