// Corpus fixture: song() sections; rendered in the linear layout and (as song-clip) the clip layout.
const meta = {
  title: 'song-linear',
  bpm: 120,
  meter: '4/4',
  sections: [
    { name: 'intro', bars: 2 },
    { name: 'body', bars: 4, loop: true },
    { name: 'tail', bars: 2 },
  ],
  loops: 2,
};

$: song(meta, {
  intro: note("c3 e3 g3 c4").s('triangle'),
  body: note("<c2 g1>*4").s('sawtooth').lpf(700),
  tail: note("c3").s('supersaw').release(2),
})
