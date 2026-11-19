// Micro case: supersaw with unison(1): one voice (panspread 0, so sqrt(0.5) per side), its phase
// drawn from mulberry32(voiceSeed)() in the first active quantum (the first quantum whose start
// time is after the float32 begin), the polyBLEP sawtooth, the 0.3 / sqrt(1) envelope peak and the
// stereo output. Two notes, one after the other. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 e4").s('supersaw').unison(1)
