// Micro case: buffer-source start frames (TimeToSampleFrame(t, RoundUp): t * sr rounded to the
// nearest 1/1024 frame, then ceil). The bars last 11025.00023 frames, so the second bar's onset
// rounds down onto frame 11025 (a tiny positive start offset: no skip, the integer fast path)
// while the half-bar onsets start a frame later and skip 0.4999 of a frame. Two 0.25 s bars.
const meta = { bpm: 239.999995, meter: '1/4', sections: [{ name: 'a', bars: 2 }] };

$: s("white brown").decay(0.04).sustain(0)
