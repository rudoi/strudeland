// Micro case: white noise voices, the run's cached buffer (makeRandom('noise', 'white', 44100)
// from meta.seed 0) played by looping AudioBufferSourceNodes: onsets on a frame (0, 11025) and
// between frames (16537.5, where Chromium skips the fractional start and interpolates linearly
// from then on). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: s("white ~ white white").decay(0.05).sustain(0.3)
