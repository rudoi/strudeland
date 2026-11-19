// Micro case: a reverb with the nested IR lowpass (roomlp 6000 ramping to roomdim 400 at the
// 0.25 s decay time) and a short fade-in (roomfade 0.02). One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("e4 ~ ~ ~").s('triangle').decay(0.05).sustain(0).room(0.8).roomsize(0.25).roomfade(0.02).roomlp(6000).roomdim(400)
