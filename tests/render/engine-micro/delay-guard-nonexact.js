// Micro case: getDelay's guard with values not exact in float (`.value` never equals them), so
// every hap schedules: bar 0 goes 0.03 s, 0.05 s, back to 0.03 s (feedback 0.3, 0.6, 0.3); bar 1
// schedules 0.03 s and 0.3 again. Two 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: note("<[a4 c5 e5 ~] [g4 ~ ~ ~]>").s('sine').decay(0.03).sustain(0).delay(0.8)
  .delaytime("<[0.03 0.05 0.03 0.03] 0.03>")
  .delayfeedback("<[0.3 0.6 0.3 0.3] 0.3>")
