// Corpus fixture: orbits 0 and 1 share stem pair 0, so the stems gate is unverifiable.
const meta = {
  title: 'stems-shared',
  bpm: 120,
  meter: '4/4',
};

$: note("c3*2").s('sine').decay(0.2).sustain(0).orbit(0)
$: note("e3*2").s('sine').decay(0.2).sustain(0).orbit(1)
