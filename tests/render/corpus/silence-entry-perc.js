// Corpus fixture: opening timing, percussive.
const meta = {
  title: 'silence-entry-perc',
  bpm: 120,
  meter: '4/4',
};

$: s("sine").note("c2*4").decay(0.15).sustain(0).mask("<0 0 1 1>")
