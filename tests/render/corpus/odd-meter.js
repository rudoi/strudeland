// Corpus fixture: 7/8 at 137 bpm (cps 0.6524).
const meta = {
  title: 'odd-meter',
  bpm: 137,
  meter: '7/8',
};

$: note("c2 ~ c2 ~ c2 ~ ~").s('sine')
$: s("white*7").hpf(7000).decay(0.03).sustain(0).gain(0.2).orbit(2)
