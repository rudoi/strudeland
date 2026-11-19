// Corpus fixture: 6/8 with overlapping bass voices and a crowded bank of quiet upper orbits.
const meta = {
  title: 'crowded-bass',
  bpm: 150,
  meter: '6/8',
};

$: note("d2 ~ d2 ~ d2 ~").s('sine').decay(0.14).sustain(0).gain(0.36).orbit(0)
$: note("~ ~ a2 ~ ~ ~").s('triangle').decay(0.14).sustain(0).gain(0.16).orbit(1)
// Cancel most of one onset, with the rest of the bass grid intact.
$: note("d2 ~ ~ ~ ~ ~").mask("<0 0 0 0 0 1 0 0>").s('user').partials([1]).phases([0.5])
  .decay(0.14).sustain(0).gain(0.32).orbit(2)
$: stack(...Array.from({ length: 45 }, (_, i) =>
  note(67 + i % 12).s(i % 2 === 0 ? 'triangle' : 'sine').struct("~ x ~ ~ ~ ~")
    .decay(0.025).sustain(0).gain(0.003).orbit(i + 3)
))
