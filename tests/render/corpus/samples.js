// Corpus fixture: the committed one-shots (tests/render/corpus/samples/), speed 1.5 from bar 2.
const meta = {
  title: 'samples',
  bpm: 120,
  meter: '4/4',
};

await samples({ kick: 'kick.wav', hat: 'hat.wav', tone: 'tone.wav' }, '/samples/');

$: s("kick*4, hat*8, ~ tone ~ ~").gain(0.6).speed("<1 1.5>")
