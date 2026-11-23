const meta = {
  bpm: 120,
  meter: '4/4',
  sections: [
    { name: 'opening', bars: 1 },
    { name: 'F1', bars: 3 },
    { name: 'F2', bars: 3 },
    { name: 'E', bars: 4 },
    { name: 'ending', bars: 1 },
  ],
  loop: { from: 'F1', through: 'E' },
  meterChanges: [
    { bar: 4, meter: '3/8' },
    { bar: 5, meter: '4/4' },
    { bar: 7, meter: '3/8' },
    { bar: 8, meter: '4/4' },
  ],
  loops: 2,
};

const f = note('f3').s('sine').clip(0.4);
$: song(meta, {
  opening: note('c4').s('sine').clip(0.4),
  F1: f,
  F2: f,
  E: note('e3').s('sine').clip(0.4),
  ending: note('g3').s('sine').clip(0.4),
});
