// Micro case: delaytime above maxDelayTime. 1.5 s renders as exactly 1 s (the param clips to
// [0, 1]); the next hap's 1 s equals `.value` (1) and is skipped. Echoes land at 1 s and 1.069 s.
// One 1.1 s bar.
const meta = { bpm: 218.18181818181819, meter: '4/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a4 e5 ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~ ~").s('sine').decay(0.02).sustain(0).delay(0.9)
  .delaytime("1.5 1").delayfeedback(0.5)
