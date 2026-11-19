// Micro case: the delay time steps while the line holds signal (a-rate, sample-accurate steps at
// onsets inside a quantum), including 0.002 s (88.2 frames: shorter than a quantum, read from the
// quantum just written). Only the delay is heard (dry 0). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 c4 e4 g4 b4").s('sawtooth').clip(0.8).dry(0).delay(1).delayfeedback(0.45)
  .delaytime("0.02 0.035 0.002 0.05 0.011")
