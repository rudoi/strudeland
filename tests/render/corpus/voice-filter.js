// Corpus fixture: filtered voices for the Rust engine: lp, hp and bp with resonance, '24db', a
// numeric ftype, filter envelopes with fanchor and a negative lpenv.
const meta = {
  title: 'voice-filter',
  bpm: 120,
  meter: '4/4',
};

$: note("a2 e3 a2 c3").s('sawtooth').lpf("<300 800 2000 5000>").lpq("<1 6 12 3>").gain(0.4)
$: note("<~ [c4 ~ e4 ~] ~ [g4 ~ ~ ~]>").s('square').hpf("<1200 3000>").hpq(8).gain(0.3)
$: note("<~ ~ [a3 e4] [d4 ~]>").s('sawtooth').bpf("<500 1500>").bpq("<2 10>").gain(0.5)
$: note("<~ ~ ~ ~ [a2 e3] [e2 b2]>").s('sawtooth').lpf(600).lpq(3).ftype('24db').gain(0.4)
$: note("<~ ~ ~ ~ ~ ~ [a2 e3 a3 c4] ~>").s('sawtooth').lpf(700).lpq(10).ftype("0 2 5 1").gain(0.3)
$: note("<~ ~ ~ ~ [~ a2 ~ e3] ~ [~ ~ ~ ~] [a2 c3 e3 a3]>").s('sawtooth').lpf(300).lpq(4).lpenv(4).lpattack(0.05).lpdecay(0.1).lpsustain(0.3).lprelease(0.1).gain(0.4)
$: note("<~ ~ ~ ~ ~ [~ ~ c3 ~] ~ [~ g3 ~ ~]>").s('square').lpf(2000).lpenv(-3).fanchor(0.5).gain(0.3)
$: note("<~ ~ ~ ~ ~ [~ c5 ~ ~] [e5 ~ ~ ~] ~>").s('sawtooth').hpf(800).hpenv(2).hpattack(0.02).hpdecay(0.1).bpf(1500).bpenv(-1.5).fanchor(0.2).gain(0.3)
