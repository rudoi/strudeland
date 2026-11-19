// Micro case: a compressor's onset state depends on the silence it processed. Voice A (bar 0,
// onset 0.125 s) processes silence from its pause frame 0 until its onset (the detector warms up
// towards 1, the gain dives and recovers). Voice B (bar 6, paused at 1.45 s, onset 1.6875 s)
// processes silence from its pause only through the quantum at frame 71808 and freezes there:
// from 71936 on Blink's silence rule (0 + 264/44100 + 1.625 s) stops processing a silent input.
// B's release of 1 s keeps its gain recovering slowly through the warm-up, so the quantum it
// freezes at shows in its onset (with the 0.05 s release the state has converged by then).
// Eight 0.25 s bars; the two voices never sound together.
const meta = { bpm: 240, meter: '1/4', sections: [{ name: 'a', bars: 8 }] };

$: note("<[~ a2] ~ ~ ~ ~ ~ ~ ~>").s('sawtooth').compressor(-20)
$: note("<~ ~ ~ ~ ~ ~ [~ ~ ~ ~ ~ ~ a2 ~] ~>").s('sawtooth').compressor(-20).compressorRelease(1)
