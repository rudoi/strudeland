// Micro case: the gain stage, gain * velocity (default gain 0.8, velocity 1; gain 0 is silent).
// The triangle sits inside the third sine, clear of note boundaries, so at most two voices sound
// at once. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 c4 e4 g4").s('sine').gain("0.5 1.2 0.8 0").velocity("1 0.5 0.25 1")
$: note("~@9 c5@2 ~@5").s('triangle').velocity(0.3)
