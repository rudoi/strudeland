// Corpus fixture: the headless host shims (visual and sample-map calls) and perlin's ** operator.
// Every pattern sounds with synth sounds; the sample map is registered but never played.
const meta = {
  title: 'host-shims',
  bpm: 120,
  meter: '4/4',
};

samples({ bd: ['bd/0.wav'] }, 'https://example.invalid/')
aliasBank({ mybank: 'alt' })

$: note("c3 e3").s('sine').gain(0.2).scope()
$: note("g3 b3").s('triangle').gain(0.2)._scope()
$: note("c4 e4").s('sine').gain(0.2)._pianoroll()
$: note("c4 e4").s('sawtooth').gain(0.1).pianoroll()
$: note("a3").s('sine').gain(slider(0.2, 0, 1))
$: note("d3 f3").s('sine').gain(0.2).pan(perlin.range(0.2, 0.8)).markcss('text-decoration:underline')
$: note("e3").s('sine').gain(0.2)._spectrum()
$: note("f3").s('sine').gain(0.2)._punchcard()
