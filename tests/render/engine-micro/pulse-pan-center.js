// Micro case: pan(0.5) behind a stereo source and behind a mono one: the pulse (two channels,
// L = R) takes the StereoPanner's stereo law (no loss at the centre), the triangle the mono law
// (0.7071 per side). One after the other. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~").s('pulse').pan(0.5)
$: note("~ a3").s('triangle').pan(0.5)
