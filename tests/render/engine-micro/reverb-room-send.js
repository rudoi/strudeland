// Micro case: room sends. Orbit 1: a mono voice (room 0.4, its default-lowpass 0.1 s room) and a
// panned voice with dry 0 (room 0.9, roomsize undefined: no new IR) heard only through the
// reverb, whose input becomes stereo; orbit 2 has its own 0.05 s room. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("c4 ~ e4 ~").s('triangle').decay(0.04).sustain(0).room(0.4).roomsize(0.1)
$: note("~ g4 ~ ~").s('sine').decay(0.04).sustain(0).pan(0.8).dry(0).room(0.9)
$: note("~ ~ ~ c5").s('square').decay(0.03).sustain(0).gain(0.5).room(0.5).roomsize(0.05).roomlp(0).orbit(2)
