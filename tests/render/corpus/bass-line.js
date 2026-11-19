// Corpus fixture: a bass-presence target (also a mutation target); every bass note is at least a beat apart.
const meta = {
  title: 'bass-line',
  bpm: 120,
  meter: '4/4',
};

$: note("<c2 g1 a1 f1>")
  .s('sine')
  .attack(0.01)
  .decay(0.5)
  .sustain(0.3)
  .release(0.2)
  .gain(0.5)
  .orbit(2)
$: note("c4 e4 g4 e4").s('triangle').decay(0.1).sustain(0).gain(0.3).orbit(3)
