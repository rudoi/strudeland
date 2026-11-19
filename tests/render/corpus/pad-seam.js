// Corpus fixture: a sustained pad with a long release across a seam.
const meta = {
  title: 'pad-seam',
  bpm: 120,
  meter: '4/4',
};

$: note("<c3 g3 e3 a3>").s('sawtooth').attack(0.5).release(4).gain(0.3).lpf(1200).room(0.3).roomsize(4)
$: note("<c2 g2>/2").s('sine').attack(0.2).release(3).gain(0.3).orbit(2)
