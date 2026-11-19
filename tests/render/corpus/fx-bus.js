// Corpus fixture: buses for the Rust engine: two buses fed from earlier bars than their readers,
// busgain on a feeder, the bus sound and bmod, each reader beside an ordinary voice on another orbit.
const meta = { title: 'fx-bus', bpm: 120, meter: '4/4' };

$: s('sine').note(40).bus(1).busgain(0.7).dry(0).gain(0.5).orbit(1)
$: s('sawtooth').note(52).lpf(1500).bus(2).dry(0).gain(0.3).orbit(2).mask("<0 0 1 1 1 1 1 1 1 1 1 1>")
$: s("bus ~ bus ~").n(1).gain(0.6).orbit(3).mask("<0 0 0 0 1 1 1 1 1 1 1 1>")
$: note("<[c4 ~ ~ ~] [~ ~ e4 ~]>").s('triangle').decay(0.2).sustain(0).gain(0.2).orbit(4)
$: s("~ bus").n(2).gain(0.5).orbit(5).mask("<0 0 0 0 0 0 1 1 1 1 1 1>")
$: s('sawtooth').note("a2 ~ e2 ~").lpf(600).bmod({ b: 2, dc: 0.5 }).gain(0.15).orbit(6).mask("<0 0 0 0 0 0 1 1 1 1 1 1>")
$: note("<[~ g3 ~ ~] [~ ~ ~ c4]>").s('square').decay(0.15).sustain(0).lpf(2000).gain(0.1).orbit(7)
