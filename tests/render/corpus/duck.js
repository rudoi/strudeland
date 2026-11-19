// Corpus fixture: a sidechain duck across orbits (the kick ducks the pad from bar 0).
const meta = {
  title: 'duck',
  bpm: 124,
  meter: '4/4',
};

$: s("sine")
  .note("c2*4")
  .decay(0.2)
  .sustain(0)
  .orbit(1)
  .duckorbit(2)
  .duckdepth(0.8)
  .duckattack(0.2)
$: note("<c3 g3>").s('sawtooth').lpf(900).release(1).orbit(2)
