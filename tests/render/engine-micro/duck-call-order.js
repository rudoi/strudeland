// Micro case: ducks run in call order. Two stacked ducker patterns hit at the same 0.25 s in bar 0
// (one pause); the job keeps query order for equal onsets, so the first pattern's duck (depth 0.9,
// onset 0.05, attack 0.2) is called first and the second's (depth 0.5, onset 0.02, attack 0.1)
// after it. The later call's cancelAndHoldAtTime(0.25) removes the first duck's ramps (the first
// event after 0.25 s is its fall to 0.051 at 0.30 s) and holds that fall's value at 0.25 s; only
// the second duck sounds, falling from the held value. (The planner sorts haps by onset, so a
// later-called duck with an earlier t cannot come from a job; unit tests pin that.) A DC voice on
// orbit 2 is held past the end; the duckers are silent carriers on orbit 1. Two 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 2 }] };

$: s("<one ~>").attack(0.001).sustain(1).clip(8).orbit(2)
$: s("<[~ one] ~>").gain(0).orbit(1).duckorbit(2).duckdepth(0.9).duckonset(0.05).duckattack(0.2)
$: s("<[~ one] ~>").gain(0).orbit(1).duckorbit(2).duckdepth(0.5).duckonset(0.02).duckattack(0.1)
