// Corpus fixture: modulated voices for the Rust engine: vibrato; pitch envelopes linear and
// exponential, positive and negative; FM with fmh, a chained fmi2, fmwave (incl. a noise wave) and
// FM envelopes.
const meta = {
  title: 'voice-mod',
  bpm: 120,
  meter: '4/4',
};

$: note("a3 e4 ~ ~").s('sine').vib("<8 5.5>").vibmod("<1 0.5>").gain(0.5)
$: note("<[~ ~ c4 ~] [~ ~ g3 ~]>").s('sawtooth').vib(6).vibmod(0.3).gain(0.3)
$: note("<~ [a3 ~ a4 ~] ~ [e3 ~ e4 ~]>").s("<sine triangle>").penv("<12 -12>").pcurve(0).pattack(0.1).pdecay(0.1).gain(0.5)
$: note("<~ ~ [c4 ~ c3 ~] [g3 ~ ~ ~]>").s('sine').penv("<-24 7>").pcurve(1).pattack(0.08).pdecay(0.05).psustain(0.5).prelease(0.1).gain(0.5)
$: note("<~ ~ ~ ~ [a2 ~ e3 ~] [a2 ~ c3 ~]>").s('sine').fm("<1.5 3>").fmh("<2 0.5 1.5 3>").gain(0.4)
$: note("<~ ~ ~ ~ [~ e3 ~ ~] [~ ~ ~ a3]>").s('sine').fm(1.5).fmi2(2).fmh(2).fmh2(0.5).fmattack(0.05).fmdecay(0.1).fmsustain(0.3).fmenv('lin').fmdecay2(0.2).gain(0.4)
$: note("<~ ~ ~ ~ ~ ~ [a3 ~ ~ ~] [~ e4 ~ ~]>").s('sine').fm("<0.3 1>").fmwave("<white crackle>").gain(0.4)
$: note("<~ ~ ~ ~ ~ ~ [~ ~ c4 ~] [~ ~ ~ g3]>").s('triangle').fm(0.5).fmwave('brown').fmi2(1).fmenv('exp').fmattack(0.02).fmdecay(0.2).fmsustain(0).gain(0.4)
$: note("<~ ~ ~ ~ ~ ~ ~ [c3 ~ ~ ~]>").s('sawtooth').fm(1).fmi31(2).fmwave3('square').fmh3(4).gain(0.3)
