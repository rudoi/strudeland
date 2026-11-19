// Corpus fixture: noise voices for the Rust engine: white, pink, brown, crackle at two densities,
// the noise mix on an oscillator, all from meta.seed. The noise-mix voices release past the window
// end on purpose: when one ends, superdough's teardown disconnects the pink source twice and throws
// a page error (InvalidAccessError), which render:check reports as a fixture error.
const meta = {
  title: 'voice-noise',
  bpm: 120,
  meter: '4/4',
  seed: 11,
};

$: s("<[white ~ pink ~] [brown ~ white ~]>").decay(0.2).sustain(0.2).gain(0.4)
$: s("<~ [crackle crackle ~ ~] ~ [~ ~ crackle crackle]>").density("<[0.02 2] [0.3 0.02]>").gain(1.5)
$: note("<~ ~ ~ ~ [a3 ~ ~ ~] [~ e3 ~ ~] [~ ~ c3 ~] [~ ~ ~ g3]>").s('sawtooth').noise("<0.3 0.8 1.2 0.5>").release(8).gain(0.2)
$: s("<~ ~ ~ ~ [pink*4] [brown*2] [white*8] [pink brown]>").decay(0.08).sustain(0).gain(0.3)
