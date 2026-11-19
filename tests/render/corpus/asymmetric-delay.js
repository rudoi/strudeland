// Corpus fixture: a figure crosses 11/8 bar boundaries while a feedback delay trails it.
const meta = {
  title: 'asymmetric-delay',
  bpm: 145,
  meter: '11/8',
};

$: note("g2 ~ ~ ~ g2 ~ ~ ~ ~ ~ ~").s('sine').decay(0.13).sustain(0).gain(0.35).orbit(1)
$: note("f4 ~ ~ a4 ~").slow(10 / 11).s('triangle').decay(0.08).sustain(0).gain(0.16).orbit(2)
  .delay(0.33).delaytime(0.9).delayfeedback(0.5)
