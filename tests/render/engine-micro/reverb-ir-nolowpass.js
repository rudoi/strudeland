// Micro case: a reverb without the IR lowpass (roomlp 0): a sawtooth stab into a 0.25 s room
// (IR 16538 frames: the direct stage, the FFT stages up to 4096 taps and two 4096-tap stages with
// their pre-delays), default roomfade 0.1. One 0.5 s bar.
const meta = { bpm: 240, meter: '2/4', sections: [{ name: 'a', bars: 1 }] };

$: note("a3 ~ ~ ~").s('sawtooth').decay(0.05).sustain(0).room(0.6).roomsize(0.25).roomlp(0)
