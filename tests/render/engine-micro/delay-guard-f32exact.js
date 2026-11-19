// Micro case: getDelay's guard with values exact in float. Bar 0 (one pause): 0.0625 s creates
// the delay, 0.125 s is scheduled, then 0.0625 s is skipped (`.value` still reads 0.0625: no
// quantum rendered in between), so the delay stays 0.125 s; the feedback 0.5, 0.25, 0.5 likewise
// stays 0.25. Bar 1: `.value` now reads 0.125 and 0.25, so 0.0625 and 0.5 are scheduled. Two 0.25
// s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("<[a4 c5 e5 ~] [g4 ~ ~ ~]>").s('sine').decay(0.03).sustain(0).delay(0.8)
  .delaytime("<[0.0625 0.125 0.0625 0.0625] 0.0625>")
  .delayfeedback("<[0.5 0.25 0.5 0.5] 0.5>")
