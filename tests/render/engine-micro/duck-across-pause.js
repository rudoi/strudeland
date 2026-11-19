// Micro case: a recovery crossing pauses. Duck 1 at 0.4375 s in bar 0 (attack 0.6) recovers until
// 1.0375 s, across the pauses of bars 1 (0.45 s) and 2 (0.95 s); its events survive both. Duck 2
// at 1.0 s in bar 2 (scheduled at 0.95 s) lands inside that recovery: its cancelAndHoldAtTime sees
// the crossing ramp, holds its value at 1.0 s and the fall starts from there. A DC voice on orbit
// 2 is held past the end; the ducker is a silent carrier on orbit 1. Three 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 3 }] };

$: s("<one ~ ~>").attack(0.001).sustain(1).clip(8).orbit(2)
$: s("<[~ ~ ~ ~ ~ ~ ~ one] ~ [one ~]>").gain(0).orbit(1).duckorbit(2).duckdepth(0.6).duckattack(0.6)
