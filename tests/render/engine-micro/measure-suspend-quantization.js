// Measure item: each bar after the first is scheduled in a pause at bar line - 0.05 s, a time
// that is not a multiple of 128 frames at this tempo (Chromium truncates it to a frame and rounds
// up to the next quantum). Each bar's notes start at fractional frames; `engine:refs --write`
// records the frame at which each first sounds. Three 0.486 s bars.
const meta = { bpm: 247, meter: '2/4', sections: [{ name: 'a', bars: 3 }] };

$: note("a4 ~ ~ ~ ~ e5 ~ ~").s('sine').decay(0.04).sustain(0)
