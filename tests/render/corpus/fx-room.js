// Corpus fixture: the reverb for the Rust engine: roomfade, roomlp, roomdim, room parameters
// changing across bars (IR swaps), and room together with delay.
const meta = {
  title: 'fx-room',
  bpm: 120,
  meter: '4/4',
};

$: note("<[c4 ~ e4 ~] [g4 ~ ~ ~]>").s('triangle').decay(0.1).sustain(0).gain(0.4)
  .room(0.6).roomsize("<1 1 2 2 0.5 0.5 1.5 1.5>").roomfade("<0.1 0.1 0.5 0.5>").roomlp("<0 0 4000 4000 8000 8000 0 0>").roomdim("<0 0 0 0 500 500 0 0>").orbit(1)
$: note("<~ [~ ~ a2 ~] ~ [~ ~ e2 ~]>").s('sawtooth').decay(0.15).sustain(0).lpf(900).gain(0.4)
  .room("<0.3 0.8>").roomsize(1).roomlp(3000).roomdim(300).orbit(2)
$: note("<~ ~ ~ ~ [a4 ~ ~ ~] [~ e5 ~ ~] [c5 ~ ~ ~] ~>").s('sine').decay(0.05).sustain(0).gain(0.4)
  .room(0.5).roomsize(0.8).delay(0.5).delaytime(0.25).delayfeedback(0.4).orbit(3)
