// Corpus fixture: seeded noise, zzfx and a reverb IR under meta.seed 7.
const meta = {
  title: 'noise-seed',
  bpm: 120,
  meter: '4/4',
  seed: 7,
};

$: s("white pink brown white").decay(0.2).sustain(0).room(0.6).roomsize(5).orbit(1)
$: s("z_noise*2").gain(0.4).orbit(2)
