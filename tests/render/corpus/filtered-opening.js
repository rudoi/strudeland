// Corpus fixture: a weighted-rest cue with a slow attack and an opening filter envelope.
const meta = {
  title: 'filtered-opening',
  bpm: 20,
  meter: '4/4',
};

$: note("~@2 g5@3 ~").s('triangle').attack(1.2).release(0.6).gain(0.29).orbit(1)
  .lpf(75).lpattack(1.2).lpenv(4)
