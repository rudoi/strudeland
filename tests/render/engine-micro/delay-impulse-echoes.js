// Micro case: echoes through two orbit delays. Orbit 1: 0.0625 s (2756.25 frames: the a-rate
// kernel interpolates between two taps), feedback 0.6 (not exact in float). Orbit 2: no
// delaytime, so delaysync 3/16 cycle / cps 2 = 0.09375 s, default feedback 0.5. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a4 ~ ~ ~").s('sine').decay(0.03).sustain(0).delay(0.5).delaytime(0.0625).delayfeedback(0.6).orbit(1)
$: note("~ e5 ~ ~").s('triangle').decay(0.02).sustain(0).delay(0.7).orbit(2)
