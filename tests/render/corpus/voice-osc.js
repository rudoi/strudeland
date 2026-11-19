// Corpus fixture: oscillator voices for the Rust engine: the four waves at low, mid, high and
// near-Nyquist pitch, the three interpolation bands (very low freq), partials/n, phases, user, one,
// octave and freq.
const meta = {
  title: 'voice-osc',
  bpm: 120,
  meter: '4/4',
};

$: note("a1 a4 c8 g9").s("<sine sawtooth square triangle>").decay(0.2).sustain(0.3).release(0.05).gain(0.5)
$: freq("<~ ~ ~ ~ [20000 ~ 1 ~] [~ 2.5 ~ 5] [21000 ~ 1.7 ~] [~ 3.3 ~ 0.5]>").s("<sawtooth square triangle sine>").gain(0.4)
$: note("<[c3 e3] [g3 c4]>").s('sawtooth').n("<12 3 1 40>").decay(0.4).sustain(0.2).gain(0.3)
$: note("<~ [a3 e4] ~ [d4 a3]>").s('square').partials([1, 0.5, 0, 0.25, 0.8]).gain(0.3)
$: note("<~ ~ [e3 b3] [c3 ~]>").s('sawtooth').partials([1, 1, 1, 1, 1]).phases([0, 0.25, 0.5, 0.125, 0.9]).gain(0.3)
$: note("<~ ~ ~ ~ [a3 ~ a5 ~] [~ c4 ~ ~]>").s('user').partials([1, 0.3, 0, 0.6, 0.1]).gain(0.4)
$: note("<~ ~ ~ ~ ~ [~ ~ e4 ~] [~ ~ ~ ~] [~ g3 ~ ~]>").s('user').gain(0.4)
$: s("<~ ~ ~ ~ ~ ~ [~ one ~ ~] [~ ~ ~ one]>").decay(0.05).sustain(0).gain(0.3)
$: note("<~ ~ ~ ~ [c e g ~] ~ [a ~ c ~] ~>").s('triangle').octave("<1 -1 2 0>").gain(0.4)
$: freq("<~ ~ ~ ~ ~ [~ 110 220 ~] ~ [330 ~ ~ 55]>").s('sine').octave("<0 1>").gain(0.4)
