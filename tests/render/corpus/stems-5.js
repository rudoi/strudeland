// Corpus fixture: five single-orbit pairs (also a mutation target); every voice has attack <= 0.01 and gain >= 0.3.
const meta = {
  title: 'stems-5',
  bpm: 120,
  meter: '4/4',
};

$: note("c2*4").s('sine').decay(0.15).sustain(0).gain(0.4).orbit(2)
$: s("~ white ~ white").decay(0.08).sustain(0).hpf(1500).gain(0.4).orbit(3)
$: s("white*8").decay(0.02).sustain(0).hpf(8000).gain(0.4).orbit(4)
$: note("<c2 g1>").s('sawtooth').decay(0.4).sustain(0.2).lpf(400).gain(0.4).orbit(5)
$: note("e4 g4 b4 g4").s('triangle').decay(0.1).sustain(0).gain(0.4).orbit(6)
