// Corpus fixture: deliberately hot, so the mix clips.
const meta = {
  title: 'clipping',
  bpm: 120,
  meter: '4/4',
};

$: note("c3 e3 g3 c4").s('sawtooth').gain(6).decay(0.5).sustain(0.8)
$: s("white*4").gain(3)
