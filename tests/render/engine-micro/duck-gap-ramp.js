// Micro case: consecutive ducks after completed recoveries. The ducker hits at 0.0625 s and every
// 0.125 s after (onset 0.02, attack 0.05, so each recovery ends 0.055 s before the next hit): no
// event lies after a hit, cancelAndHoldAtTime does nothing and each fall runs from the previous
// recovery's end (1) to the hit's t + onset. duckdepth 2 floors the ducked gain at 0.01. A DC voice
// on orbit 2 is held past the end; the ducker is a silent carrier on orbit 1. Four 0.25 s bars.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 4 }] };

$: s("<one ~ ~ ~>").attack(0.001).sustain(1).clip(8).orbit(2)
$: s("[~ one]*2").gain(0).orbit(1).duckorbit(2).duckonset(0.02).duckattack(0.05).duckdepth(2)
