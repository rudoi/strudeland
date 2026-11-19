// Corpus fixture: every worklet effect, one line per effect, each on its own orbit.
// The fork has no `ladder()` function; the ladder filter worklet is selected with `ftype('ladder')`.
const meta = {
  title: 'worklets',
  bpm: 120,
  meter: '4/4',
};

$: note("c3 e3 g3 e3").s('sawtooth').shape(0.6).gain(0.25).orbit(1)
$: note("c3*4").s('sawtooth').crush(4).gain(0.25).orbit(2)
$: note("e3 g3").s('sawtooth').coarse(8).gain(0.25).orbit(3)
$: note("c2 c3").s('sawtooth').lpf(900).lpq(8).ftype('ladder').gain(0.25).orbit(4)
$: note("g2*2").s('sawtooth').distort(2).gain(0.15).orbit(5)
$: note("c3 g3").s('sawtooth').djf("<0.2 0.8>").gain(0.25).orbit(6)
$: note("c3 e3").s('supersaw').unison(5).detune(0.4).gain(0.25).orbit(7)
