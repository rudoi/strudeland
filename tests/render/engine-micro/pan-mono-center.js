// Micro case: StereoPannerNode on a mono voice (the equal-power mono law; pan 0.5 is the centre,
// 0.7071 per side). Without pan the voice is mono into the stereo mix. The sine sits inside the
// last triangle, clear of note boundaries, so at most two voices sound at once. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 e4 c4 g4").s('triangle').pan("0.5 0 1 0.25")
$: note("~@13 c5@2 ~").s('sine')
