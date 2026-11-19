// Corpus fixture: a roomsize 20 reverb tail across a seam.
const meta = {
  title: 'reverb-long',
  bpm: 100,
  meter: '4/4',
};

$: note("c4 ~ ~ ~ e4 ~ ~ ~").s('triangle').decay(0.2).sustain(0).gain(0.4).room(0.9).roomsize(20).orbit(1)
