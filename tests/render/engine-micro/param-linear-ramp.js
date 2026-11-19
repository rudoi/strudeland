// Micro case: linearRampToValueAtTime between fractional frames (Blink's float `k` and `x`).
// Every envelope segment of `one` (a constant 1) is a linear ramp whose ends fall between frames;
// the first onset is at 2756.25 frames, the second at 11025. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("~ one ~ ~ one ~ ~ ~").attack(0.0123).decay(0.0234).sustain(0.4).release(0.0345)
