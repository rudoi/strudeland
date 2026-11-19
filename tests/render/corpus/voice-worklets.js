// Corpus fixture: the synth and voice worklets for the Rust engine: supersaw (defaults, unison,
// spread, detune, pitch envelope, vibrato), pulse with PWM, transient, the ladder filter with drive
// and envelope, all nine distort types, shape, distortvol, coarse and crush, each on its own orbit.
const meta = { title: 'voice-worklets', bpm: 120, meter: '4/4' };

$: note("<c3 e3>").s('supersaw').gain(0.1).orbit(1)
$: note("<[a2 ~] [~ e3]>").s('supersaw').unison("<1 2.5 7>").spread("<0 0.5 1>").detune("<0.1 0.4>").gain(0.1).orbit(2)
$: note("<~ [g3 ~ c4 ~]>").s('supersaw').penv(7).pattack(0.05).pdecay(0.1).vib(5).vibmod(0.5).gain(0.1).orbit(3)
$: note("<[c3 ~ e3 ~] [g2 ~ ~ ~]>").s('pulse').pw("<0.2 0.5 0.8>").pwrate(3).pwsweep(0.4).gain(0.12).orbit(4)
$: note("<[c4 ~ ~ ~] [~ ~ g4 ~]>").s('triangle').transient("<0.8 -0.6>").gain(0.2).orbit(5)
$: note("<~ [~ e4 ~ ~]>").s('triangle').transient("0:-0.8").gain(0.2).orbit(5)
$: note("<[a1 ~ a2 ~] [e2 ~ ~ e1]>").s('sawtooth').lpf(700).lpq(10).ftype('ladder').drive("<1 3>").lpenv(3).lpattack(0.05).gain(0.12).orbit(6)
$: note("<c3*9 ~>").s('sawtooth').distort(2).distorttype("0 1 2 3 4 5 6 7 8").gain(0.08).orbit(7)
$: note("<~ [e3 g3]>").s('sawtooth').shape("0.7:0.5").gain(0.12).orbit(8)
$: note("<[~ c3] ~>").s('sawtooth').distort(3).distortvol(0.4).gain(0.12).orbit(9)
$: note("<[e3 ~ g3 ~] ~>").s('sawtooth').coarse("<4 8 12>").gain(0.12).orbit(10)
$: note("<~ [~ ~ c4 a3]>").s('sawtooth').crush("<4 3.5 6>").gain(0.1).orbit(11)
