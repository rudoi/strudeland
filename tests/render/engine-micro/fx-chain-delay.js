// Micro case: an FX chain entry's local delay. The entry (delay 0.5, delaytime 0.1, feedback 0.5)
// builds its own FeedbackDelay (no start(t)) after its gain stage: tail -> dry gain(1) and the
// delay line, line -> wet gainNode(0.5) (no gain curve), both summed into a 2-channel sum, so the
// voice is stereo from there. fxr(0.5) fades the echoes to exactly 0 at endWithRelease, where the
// engine cuts the tail (Chromium's releaseNodes lands later, at a varying quantum). One short note
// per bar. Three 0.5 s bars.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 3 }] };

$: note("a3 ~ ~ ~ ~ ~ ~ ~").s('sawtooth').clip(.2).fxr(.5).FX(delay(.5).delaytime(.1).delayfeedback(.5))
