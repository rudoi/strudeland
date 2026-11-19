// Micro case: an FX chain entry's local reverb. The entry (room 0.5, roomsize 0.8) builds its own
// convolver with a seeded IR (makeRandom('reverb', ...)): tail -> dry gain(1) and the convolver,
// convolver -> wet gainNode(0.5), both into a 2-channel sum. Each bar's voice builds an equal
// reverb (the engine's run-wide IR cache hit from bar 1 on). fxr(0.5) fades the tail to exactly 0
// at endWithRelease, where the engine cuts it. One short note per bar. Three 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 3 }] };

$: note("a3 ~ ~ ~ ~ ~ ~ ~").s('sawtooth').clip(.2).fxr(.5).FX(room(.5).roomsize(.8))
