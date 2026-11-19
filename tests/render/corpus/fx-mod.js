// Corpus fixture: modulators for the Rust engine at a non-default cps: lfo on gain, cutoff, pan
// and note, a 24 dB filter, a mod of a mod, an envelope on the cutoff, the three filter LFOs and a
// modulated orbit delaytime, each on its own orbit.
const meta = { title: 'fx-mod', bpm: 100, meter: '4/4' };

$: note("<a3 ~ c4 ~>").s('sine').gain(0.4).lfo({ r: 4, dep: 0.5 }).orbit(1)
$: note("<[a2 ~] ~ [e3 ~] ~>").s('sawtooth').lpf(800).lfo({ r: 0.5, dep: 0.6 }).gain(0.12).orbit(2)
$: note("<~ [c3 ~ e3 ~]>").s('sawtooth').pan(0.5).lfo({ r: 1 }).gain(0.1).orbit(3)
$: note("<48 ~ ~ 48>").s('triangle').lfo({ c: 'note', r: 6, dep: 0.02 }).gain(0.2).orbit(4)
$: note("<~ e2 ~ a2>").s('sawtooth').lpf(600).lfo({ r: 2, dep: 0.5 }).ftype('24db').gain(0.12).orbit(5)
$: note("<~ ~ [a2 ~ ~ ~] ~>").s('sawtooth').lpf(500).lfo().lfo({ sc: 'rate', da: 3 }).gain(0.12).orbit(6)
$: note("<[a2 ~ ~ ~] ~ ~ [~ ~ e3 ~]>").s('sawtooth').lpf(300).env({ a: 0.05, d: 0.1, s: 0.3, depthabs: 2000 }).gain(0.12).orbit(7)
$: note("<~ [c3 ~ ~ ~] ~ ~>").s('sawtooth').lpf(500).lprate(1).lpdepth(0.4).lpsync(1).gain(0.12).orbit(8)
$: note("<[~ ~ g3 ~] ~ ~ ~>").s('sawtooth').hpf(1200).hprate(2).hpdepth(0.3).gain(0.12).orbit(9)
$: note("<~ ~ ~ [e3 ~ ~ ~]>").s('sawtooth').bpf(1500).bprate(1.5).bpdepth(0.3).gain(0.15).orbit(10)
$: note("<[a4 ~ ~ ~] ~ [~ ~ e4 ~] ~>").s('sine').decay(0.1).sustain(0).delay(0.5).delaytime(0.2).delayfeedback(0.3).lfo({ c: 'delaytime', r: 0.5, dep: 0.1 }).gain(0.3).orbit(11)
