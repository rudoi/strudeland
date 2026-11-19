// Micro case: one ducker, two targets. duckorbit("2:3") with per-target onset and depth arrays
// ducks the DC voices on orbits 2 and 3 differently (index 0 and 1 of each array); the scalar
// duckattack is one array element, so orbit 3 falls back to attackArr[0]. The ducker (a silent
// carrier on orbit 1) hits at 0.25 s. (A target orbit no hap uses is a silent no-op in the engine,
// but superdough logs it and the harness reports that as an error, so it is pinned by unit tests
// only.) Two 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 2 }] };

$: s("<one ~>").attack(0.001).sustain(1).clip(8).orbit(2)
$: s("<one ~>").attack(0.001).sustain(1).clip(8).orbit(3).gain(0.5)
$: s("<[~ one] ~>").gain(0).orbit(1).duckorbit("2:3").duckonset("0:0.05").duckattack(0.3).duckdepth("0.8:0.5")
