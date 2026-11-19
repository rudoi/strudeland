// Corpus fixture: a mid-song room and roomsize change on one orbit (the IR is regenerated; guards the IR wait).
const meta = {
  title: 'reverb-change',
  bpm: 120,
  meter: '4/4',
};

$: note("c4 e4 g4 c5")
  .s('triangle')
  .decay(0.15)
  .sustain(0)
  .gain(0.4)
  .orbit(1)
  .room("<0.3!8 0.8!8>")
  .roomsize("<3!8 18!8>")
