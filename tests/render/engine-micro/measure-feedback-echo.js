// Measure item: the feedback cycle of superdough's delay (DelayNode -> Gain(feedback) ->
// DelayNode). A short burst goes through a 0.05 s delay (2205 frames: f32(0.05) * 44100 rounds to
// a whole frame, so no interpolation) with feedback 0.5; `engine:refs --write` records where echo
// k = 1..4 first sounds against `t0 + k*D + (k-1)*128` (each loop through the cycle adds one
// render quantum). One 0.3 s bar.
const meta = { bpm: 200, meter: '1/4', sections: [{ name: 'a', bars: 1 }] };

$: s("one").clip(0.02).delay(1).delaytime(0.05).delayfeedback(0.5)
