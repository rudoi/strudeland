// Micro case: a duck inside the previous recovery. Duck 1 at 0.25 s (depth 0.8, attack 0.4) falls
// from the pause to 0.1056 at 0.25 s and recovers until 0.65 s. Duck 2 at 0.40 s (depth 0.1, onset
// 0.03, attack 0.1), called after it in the same pause, cancels inside that recovery: a
// CancelValues event at 0.40 s keeps the saved exponential ramp and holds its float value there
// (cached when the segment before it is first rendered); the JS held value (about 0.245) clamps
// ducked = 1 - sqrt(0.1) = 0.684 down to it, and the next fall starts from the cached value. A DC
// voice on orbit 2 is held past the end; the duckers are silent carriers on orbit 1. Two 0.5 s
// bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 2 }] };

$: s("<one ~>").attack(0.001).sustain(1).clip(8).orbit(2)
$: s("<[~ one] ~>").gain(0).orbit(1).duckorbit(2).duckdepth(0.8).duckattack(0.4)
$: s("<[~ ~ ~ ~ ~ ~ ~ ~ one ~] ~>").gain(0).orbit(1).duckorbit(2).duckdepth(0.1).duckonset(0.03).duckattack(0.1)
