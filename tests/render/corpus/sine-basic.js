// Corpus fixture: plain sine voices end to end (the first Rust-engine pipeline fixture): notes
// with the default envelope and one with an explicit attack and release.
const meta = {
  title: 'sine-basic',
  bpm: 120,
  meter: '4/4',
};

$: note("<c3 a2 f2 g2>").s('sine').gain(0.6)
$: note("e4 g4 ~ b4").s('sine')
$: note("~ ~ c5 ~").s('sine').attack(0.08).release(0.3)
