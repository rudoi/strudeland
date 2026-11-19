// Corpus fixture: a dense 9/8 schedule with a late render window, feedback and sidechain.
const meta = {
  title: 'late-feedback',
  bpm: 180,
  meter: '9/8',
};

$: note("d2*3").s('sine').decay(0.1).sustain(0).gain(0.36).orbit(2)
  .duckorbit(4).duckdepth(0.45).duckattack(0.08)
$: note("<f3 a3 e3 b3>*9").s('supersaw').attack(0.005).decay(0.06).sustain(0)
  .release(0.12).lpf(1200).gain(0.13).orbit(4)
$: note("d5*18").s('triangle').decay(0.019).sustain(0).gain(0.06).orbit(6)
  .delay(0.21).delaytime(0.29).delayfeedback(0.26)
$: note("<a5 g5>*18").s('sine').decay(0.017).sustain(0).gain(0.035).orbit(8)
  .room(0.19).roomsize(3)
