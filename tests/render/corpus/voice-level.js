// Corpus fixture: voice levels for the Rust engine: a pan sweep, velocity, postgain, dry, clip and
// the ADSR default rules (attack only, sustain only, decay only: the sustain 0.001 rule).
const meta = {
  title: 'voice-level',
  bpm: 120,
  meter: '4/4',
};

$: note("c4*8").s('sawtooth').pan(saw.range(0, 1)).decay(0.1).sustain(0.2).gain(0.4)
$: note("<[a3 e4 a3 e4] [c4 g4 c4 g4]>").s('triangle').velocity("1 0.5 0.25 0.75").gain(0.5)
$: note("<~ [a3 ~] ~ [e3 ~]>").s('sawtooth').postgain("<0.5 2>").dry("<0.7 0.4>").gain(0.3)
$: note("<~ ~ [c3 e3 g3 ~] [g2 ~ d3 ~]>").s('sine').clip("<0.5 1.5>").gain(0.5)
$: note("<~ ~ ~ ~ [a3 ~ ~ ~] ~ ~ ~>").s('sine').attack(0.2).gain(0.5)
$: note("<~ ~ ~ ~ ~ [e4 ~ ~ ~] ~ ~>").s('sine').sustain(0.4).gain(0.5)
$: note("<~ ~ ~ ~ ~ ~ [c4 ~ c4 ~] ~>").s('sine').decay(0.2).gain(0.5)
$: note("<~ ~ ~ ~ ~ ~ ~ [g3 ~ ~ ~]>").s('triangle').attack(0.05).decay(0.1).sustain(0.5).release(0.3).pan(0.8).postgain(1.5).dry(0.4).gain(0.4)
