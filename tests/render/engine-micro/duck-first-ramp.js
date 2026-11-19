// Micro case: the first duck on an orbit. A DC voice on orbit 2 is held past the end. The ducker
// (a silent carrier on orbit 1, gain 0) at 0.75 s in bar 1 is scheduled at bar 1's pause (0.45 s,
// frame 19968): the output gain's timeline is empty, so cancelAndHoldAtTime does nothing and the
// first exponential ramp inserts setValueAtTime(1, pause). The gain falls from 1 at the pause frame
// to 1 - sqrt(0.75) = 0.134 at t + onset = 0.75 s and recovers to 1 at 1.05 s. Three 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 3 }] };

$: s("<one ~ ~>").attack(0.001).sustain(1).clip(8).orbit(2)
$: s("<~ [~ one] ~>").gain(0).orbit(1).duckorbit(2).duckdepth(0.75).duckattack(0.3)
