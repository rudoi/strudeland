// Corpus fixture: opening timing, soft attack.
const meta = {
  title: 'silence-entry-pad',
  bpm: 120,
  meter: '4/4',
};

$: note("c3").s('sawtooth').attack(0.8).release(1).lpf(900).mask("<0 1 1 1>")
