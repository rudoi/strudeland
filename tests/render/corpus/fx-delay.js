// Corpus fixture: the feedback delay for the Rust engine: delaytime, delaysync at a non-default
// cps, feedback changes inside a tail (f32-exact and non-exact values for the guard), two orbits.
const meta = {
  title: 'fx-delay',
  bpm: 100,
  meter: '4/4',
};

$: note("<[a4 ~ ~ ~] [c5 ~ ~ ~] [e4 ~ ~ ~] [g4 ~ ~ ~]>").s('triangle').decay(0.05).sustain(0).gain(0.5)
  .delay(0.6).delaytime("<0.25 0.375>").delayfeedback("<0.5 0.25 0.3 0.6>").orbit(1)
$: note("<~ [~ ~ e3 ~] ~ [~ ~ b2 ~]>").s('sawtooth').decay(0.08).sustain(0).lpf(1200).gain(0.4)
  .delay(0.5).delaysync("<0.25 0.1875>").delayfeedback(0.45).orbit(2)
$: note("<~ ~ ~ ~ [c4 e4 ~ ~] [~ ~ ~ ~] [g4 ~ ~ ~] ~>").s('sine').decay(0.04).sustain(0).gain(0.5)
  .delay(0.7).delaytime(0.3).delayfeedback("<0.5 0.5 0.6 0.25>").orbit(1)
