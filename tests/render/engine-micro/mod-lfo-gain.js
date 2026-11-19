// Micro case: an lfo modulator on the gain stage (`gain(0.5).lfo({ r: 8 })`, default control
// gain): currentValue is the f32 gain 0.5, depth 1 * 0.5, min/max getLfo's defaults (-0.25,
// 0.25), time = cycle / cps (the second hap at cycle 0.5), connected from the pause frame (the
// a-rate path of the gain from scheduling) and the LFO start quantum (the first quantum whose
// currentTime is past the f32 begin). Two 0.25 s notes, one after the other. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 c4").s('sine').gain(0.5).lfo({ r: 8 })
